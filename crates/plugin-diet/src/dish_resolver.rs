//! Dish matching: exact match then candidates + one LLM pick per meal, per
//! `docs/DISH_NUTRITION.md` §5.2–§5.4.

use lifeagent_core::plugin::{PluginContext, PluginError};
use lifeagent_llm::json::complete_json_once;
use lifeagent_llm::types::{Message, Role};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::foods::{dice_score, normalize};
use crate::model::Dish;
use crate::parser::ParsedDish;
use crate::prompts::DISH_PICK;
use crate::sources::SourceChain;

/// Local dish rows cut to this many candidates by bigram score before the pick call (§5.3
/// step 1).
const MAX_CANDIDATES: usize = 200;

/// A parsed dish that matched a per-serving record, ready to become a dish `MealItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchedDish {
    pub dish: Dish,
    pub servings: f64,
    /// `serving_g * servings` when `serving_g` is known, else `None` (§6).
    pub grams: Option<f64>,
}

/// The outcome of resolving one parsed dish (§5.2–§5.4): a per-serving match, or a dish left for
/// the caller to decompose into its components (§5.5).
#[derive(Debug, Clone, PartialEq)]
pub enum DishOutcome {
    Matched(MatchedDish),
    Unmatched(ParsedDish),
}

/// Resolves every parsed dish, in order. Stage 2 (exact match) runs per dish; stage 3
/// (candidates + pick) runs at most one model call for the whole meal, covering every branded
/// exact miss that has local rows for its brand (§5.3).
pub async fn resolve_dishes(
    ctx: &PluginContext,
    dishes: Vec<ParsedDish>,
) -> Result<Vec<DishOutcome>, PluginError> {
    let chain = SourceChain::load(ctx).await?;

    // Stage 2: exact match only. Whether a dish with `grams` and no `serving_g` falls back
    // (§5.4) is decided below, once, from this — an exact hit must never also try stage 3.
    let mut resolved: Vec<Option<Dish>> = Vec::with_capacity(dishes.len());
    for pd in &dishes {
        let brand = pd.brand.as_deref().unwrap_or("");
        resolved.push(exact_match(&chain, brand, pd).await?);
    }

    // Stage 3 candidates: only branded exact misses with local rows for their brand.
    let mut pick_targets: Vec<(usize, String, Vec<Dish>)> = Vec::new();
    for (i, pd) in dishes.iter().enumerate() {
        if resolved[i].is_some() {
            continue;
        }
        let Some(brand) = pd.brand.as_deref() else {
            continue;
        };
        let brand_key = normalize(brand);
        let candidates = chain.brand_candidates(&brand_key).await?;
        if !candidates.is_empty() {
            let ranked = rank_candidates(candidates, &query_string(pd));
            pick_targets.push((i, brand_key, ranked));
        }
    }

    if !pick_targets.is_empty() {
        // §5.3 step 3: a failed or wrong-shaped pick just means decomposition, never a failed
        // meal log. The error text can quote model output, so it stays at DEBUG.
        let picks = match run_pick(ctx, &dishes, &pick_targets).await {
            Ok(picks) => picks,
            Err(e) => {
                tracing::warn!("dish pick failed; falling back to components");
                tracing::debug!(error = %e, "dish pick error");
                vec![None; pick_targets.len()]
            }
        };
        for (target_idx, pick) in picks.into_iter().enumerate() {
            let Some(candidate_idx) = pick else {
                continue;
            };
            let (dish_idx, brand_key, candidates) = &pick_targets[target_idx];
            let Some(dish) = candidates.get(candidate_idx).cloned() else {
                continue;
            };
            write_pick_alias(ctx, brand_key, &dishes[*dish_idx], &dish.name_key).await?;
            resolved[*dish_idx] = Some(dish);
        }
    }

    // Stage 4's grams-without-serving_g case (§5.4) is applied last, uniformly for both an
    // exact hit and a pick hit.
    Ok(dishes
        .into_iter()
        .zip(resolved)
        .map(
            |(pd, dish)| match dish.and_then(|d| build_matched(d, &pd)) {
                Some(m) => DishOutcome::Matched(m),
                None => DishOutcome::Unmatched(pd),
            },
        )
        .collect())
}

/// The text a dish is matched/ranked/aliased against: `"{name} {size}"`, or just `name` without
/// a stated size.
fn query_string(pd: &ParsedDish) -> String {
    match pd.size.as_deref().filter(|s| !s.is_empty()) {
        Some(size) => format!("{} {size}", pd.name),
        None => pd.name.clone(),
    }
}

/// §5.2: `lookup_dish` runs twice at most — first with `"{name} {size}"` when a size was stated,
/// then with `name` alone.
async fn exact_match(
    chain: &SourceChain,
    brand: &str,
    pd: &ParsedDish,
) -> Result<Option<Dish>, PluginError> {
    if let Some(size) = pd.size.as_deref().filter(|s| !s.is_empty()) {
        if let Some(dish) = chain
            .lookup_dish(brand, &format!("{} {size}", pd.name))
            .await?
        {
            return Ok(Some(dish));
        }
    }
    chain.lookup_dish(brand, &pd.name).await
}

/// §5.4's extra case: a matched dish with `grams` stated converts through `serving_g` when
/// known; when `serving_g` is unknown the grams can't be trusted, so the dish falls back to its
/// components (`None`, logged at DEBUG).
fn build_matched(dish: Dish, pd: &ParsedDish) -> Option<MatchedDish> {
    match pd.grams {
        Some(grams) => match dish.serving_g {
            Some(serving_g) if serving_g > 0.0 => Some(MatchedDish {
                servings: round_servings(grams / serving_g),
                grams: Some(grams),
                dish,
            }),
            _ => {
                tracing::debug!(
                    dish = dish.name,
                    "matched dish has grams but no serving_g; falling back to components"
                );
                None
            }
        },
        None => {
            let grams = dish.serving_g.map(|g| g * pd.servings);
            Some(MatchedDish {
                servings: pd.servings,
                grams,
                dish,
            })
        }
    }
}

/// Servings derived from grams are shown to the user ("×1.14"), and the item's macros are
/// computed from the same rounded value so the two agree. Never rounds down to zero.
fn round_servings(servings: f64) -> f64 {
    ((servings * 100.0).round() / 100.0).max(0.01)
}

/// §5.3 step 1: cuts more than 200 candidates to the top 200 by character-bigram Dice score
/// against `query`, ties broken by `use_count`.
fn rank_candidates(candidates: Vec<Dish>, query: &str) -> Vec<Dish> {
    if candidates.len() <= MAX_CANDIDATES {
        return candidates;
    }
    let mut scored: Vec<(f64, i64, Dish)> = candidates
        .into_iter()
        .map(|d| (dice_score(&d.name, query), d.use_count, d))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    scored
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, _, d)| d)
        .collect()
}

#[derive(Deserialize)]
struct RawPicks {
    #[serde(default)]
    picks: Vec<RawPick>,
}

#[derive(Deserialize)]
struct RawPick {
    dish: usize,
    #[serde(default)]
    candidate: Option<usize>,
}

/// One `DISH_PICK` call covering every pending dish in the meal (§5.3 step 2). Returns, per
/// pending dish (by position in `pick_targets`), the chosen candidate index or `None`.
async fn run_pick(
    ctx: &PluginContext,
    dishes: &[ParsedDish],
    pick_targets: &[(usize, String, Vec<Dish>)],
) -> Result<Vec<Option<usize>>, PluginError> {
    let entries: Vec<Value> = pick_targets
        .iter()
        .enumerate()
        .map(|(pick_idx, (dish_idx, _brand_key, candidates))| {
            let pd = &dishes[*dish_idx];
            json!({
                "dish": pick_idx,
                "name": pd.name,
                "size": pd.size,
                "servings": pd.servings,
                // Numbered explicitly: the model shouldn't count positions in a 200-name list.
                "candidates": candidates
                    .iter()
                    .enumerate()
                    .map(|(i, d)| json!({"index": i, "name": d.name}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let pending_json =
        serde_json::to_string(&entries).map_err(|e| PluginError::Other(e.to_string()))?;
    let prompt = DISH_PICK.replace("{pending_dishes}", &pending_json);

    let raw: RawPicks = complete_json_once(
        &ctx.llm,
        Role::Parser,
        vec![Message::User { content: prompt }],
    )
    .await?;

    // §5.3 step 3: an out-of-range index, or a dish missing from the output, counts as null.
    let mut result = vec![None; pick_targets.len()];
    for pick in raw.picks {
        if pick.dish >= result.len() {
            continue;
        }
        let candidate_count = pick_targets[pick.dish].2.len();
        result[pick.dish] = pick.candidate.filter(|&c| c < candidate_count);
    }
    Ok(result)
}

/// §5.3 step 4: writes the pick as a redirect so the same wording hits stage 2 next time. Only
/// called for an actual pick, so a `null` is never cached (§11 Q1).
async fn write_pick_alias(
    ctx: &PluginContext,
    brand_key: &str,
    pd: &ParsedDish,
    target_name_key: &str,
) -> Result<(), PluginError> {
    let alias_key = normalize(&query_string(pd));
    if alias_key == target_name_key {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO diet_dish_name_aliases (brand_key, alias_key, target_name_key, origin, created_at)
         VALUES ($1, $2, $3, 'pick', $4)
         ON CONFLICT(brand_key, alias_key) DO UPDATE SET
            target_name_key = excluded.target_name_key, origin = excluded.origin,
            created_at = excluded.created_at",
    )
    .bind(brand_key)
    .bind(&alias_key)
    .bind(target_name_key)
    .bind(ctx.clock.now().to_rfc3339())
    .execute(&ctx.db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ParsedComponent;
    use crate::sources::upsert_manual_dish;
    use crate::testutil::{env, json_response, RICE};
    use lifeagent_core::clock::Clock;
    use serde_json::json;

    fn dish_with(
        name: &str,
        brand: Option<&str>,
        size: Option<&str>,
        servings: f64,
        grams: Option<f64>,
    ) -> ParsedDish {
        ParsedDish {
            name: name.into(),
            brand: brand.map(String::from),
            size: size.map(String::from),
            servings,
            grams,
            components: vec![ParsedComponent {
                name: "米饭".into(),
                grams: 200.0,
                alt_names: vec![],
            }],
        }
    }

    #[tokio::test]
    async fn exact_hit_by_name_and_size_makes_no_llm_call() {
        let env = env().await;
        // Stored as "{name} {size}", matching how stage 2 combines the parsed dish's own name
        // and size (§5.2) — this is the case that exercises that combination.
        upsert_manual_dish(&env.ctx, "牛めし 並盛", Some("松屋"), RICE, Some(350.0))
            .await
            .unwrap();
        let pd = dish_with("牛めし", Some("松屋"), Some("並盛"), 2.0, None);

        let outcomes = resolve_dishes(&env.ctx, vec![pd]).await.unwrap();

        let DishOutcome::Matched(m) = &outcomes[0] else {
            panic!("expected a match");
        };
        assert_eq!(m.dish.name, "牛めし 並盛");
        assert_eq!(m.servings, 2.0);
        assert_eq!(m.grams, Some(700.0));
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn brandless_dish_never_matches_a_branded_one_and_no_pick_is_attempted() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛丼", Some("松屋"), RICE, None)
            .await
            .unwrap();
        let pd = dish_with("牛丼", None, None, 1.0, None);

        let outcomes = resolve_dishes(&env.ctx, vec![pd.clone()]).await.unwrap();

        assert_eq!(outcomes, vec![DishOutcome::Unmatched(pd)]);
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn an_unknown_brand_with_no_local_rows_decomposes_without_a_pick_call() {
        let env = env().await;
        let pd = dish_with("巨无霸", Some("麦当劳"), None, 1.0, None);

        let outcomes = resolve_dishes(&env.ctx, vec![pd.clone()]).await.unwrap();

        assert_eq!(outcomes, vec![DishOutcome::Unmatched(pd)]);
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn pick_hit_logs_the_candidate_and_writes_an_alias_for_next_time() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, Some(350.0))
            .await
            .unwrap();
        env.mock.push_response(json_response(
            json!({"picks": [{"dish": 0, "candidate": 0}]}),
        ));
        let pd = dish_with("牛肉饭", Some("松屋"), None, 1.0, None);

        let outcomes = resolve_dishes(&env.ctx, vec![pd.clone()]).await.unwrap();

        let DishOutcome::Matched(m) = &outcomes[0] else {
            panic!("expected a match");
        };
        assert_eq!(m.dish.name, "牛めし（並盛）");
        assert_eq!(env.mock.received_requests().len(), 1);

        // The alias was learned: a second identical log needs no pick call.
        let outcomes2 = resolve_dishes(&env.ctx, vec![pd]).await.unwrap();
        let DishOutcome::Matched(m2) = &outcomes2[0] else {
            panic!("expected a match on the second log");
        };
        assert_eq!(m2.dish.name, "牛めし（並盛）");
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn pick_prompt_maps_a_generic_size_to_the_regular_one() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, Some(350.0))
            .await
            .unwrap();
        env.mock.push_response(json_response(
            json!({"picks": [{"dish": 0, "candidate": 0}]}),
        ));
        let pd = dish_with("牛肉饭", Some("松屋"), Some("中碗"), 1.0, None);

        resolve_dishes(&env.ctx, vec![pd]).await.unwrap();

        let request = &env.mock.received_requests()[0];
        let Message::User { content } = &request.messages[0] else {
            panic!("expected a user message");
        };
        assert!(content.contains("中碗"));
        assert!(content.contains("regular size"));

        let alias: Option<(String,)> = sqlx::query_as(
            "SELECT target_name_key FROM diet_dish_name_aliases WHERE brand_key = $1 AND alias_key = $2",
        )
        .bind(normalize("松屋"))
        .bind(normalize("牛肉饭 中碗"))
        .fetch_optional(&env.ctx.db)
        .await
        .unwrap();
        assert_eq!(alias.unwrap().0, normalize("牛めし（並盛）"));
    }

    #[tokio::test]
    async fn null_out_of_range_and_missing_picks_all_decompose() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, None)
            .await
            .unwrap();
        // Three dishes: null pick, out-of-range candidate, and one missing from the output.
        env.mock.push_response(json_response(json!({
            "picks": [{"dish": 0, "candidate": null}, {"dish": 1, "candidate": 99}]
        })));
        let pds = vec![
            dish_with("A饭", Some("松屋"), None, 1.0, None),
            dish_with("B饭", Some("松屋"), None, 1.0, None),
            dish_with("C饭", Some("松屋"), None, 1.0, None),
        ];

        let outcomes = resolve_dishes(&env.ctx, pds.clone()).await.unwrap();

        assert_eq!(
            outcomes,
            vec![
                DishOutcome::Unmatched(pds[0].clone()),
                DishOutcome::Unmatched(pds[1].clone()),
                DishOutcome::Unmatched(pds[2].clone()),
            ]
        );
        // All three shared one call.
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn a_wrong_shaped_pick_decomposes_without_a_repair_call() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, None)
            .await
            .unwrap();
        env.mock
            .push_response(json_response(json!({"picks": "not a list"})));
        let pd = dish_with("牛肉饭", Some("松屋"), None, 1.0, None);

        let outcomes = resolve_dishes(&env.ctx, vec![pd.clone()]).await.unwrap();

        assert_eq!(outcomes, vec![DishOutcome::Unmatched(pd)]);
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_pick_call_decomposes_instead_of_failing_the_meal() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, None)
            .await
            .unwrap();
        env.mock.push_error(lifeagent_llm::error::LlmError::Auth);
        let pd = dish_with("牛肉饭", Some("松屋"), None, 1.0, None);

        let outcomes = resolve_dishes(&env.ctx, vec![pd.clone()]).await.unwrap();

        assert_eq!(outcomes, vec![DishOutcome::Unmatched(pd)]);
    }

    #[tokio::test]
    async fn pick_prompt_numbers_every_candidate_explicitly() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, None)
            .await
            .unwrap();
        upsert_manual_dish(&env.ctx, "牛めし（大盛）", Some("松屋"), RICE, None)
            .await
            .unwrap();
        env.mock.push_response(json_response(
            json!({"picks": [{"dish": 0, "candidate": null}]}),
        ));
        let pd = dish_with("牛肉饭", Some("松屋"), None, 1.0, None);

        resolve_dishes(&env.ctx, vec![pd]).await.unwrap();

        let request = &env.mock.received_requests()[0];
        let Message::User { content } = &request.messages[0] else {
            panic!("expected a user message");
        };
        assert!(content.contains(r#"{"index":0,"name":"#));
        assert!(content.contains(r#"{"index":1,"name":"#));
    }

    #[tokio::test]
    async fn two_branded_misses_in_one_meal_share_one_pick_call() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, None)
            .await
            .unwrap();
        upsert_manual_dish(&env.ctx, "牛丼（並）", Some("吉野家"), RICE, None)
            .await
            .unwrap();
        env.mock.push_response(json_response(json!({
            "picks": [{"dish": 0, "candidate": 0}, {"dish": 1, "candidate": 0}]
        })));
        let pds = vec![
            dish_with("牛肉饭", Some("松屋"), None, 1.0, None),
            dish_with("牛肉饭", Some("吉野家"), None, 1.0, None),
        ];

        let outcomes = resolve_dishes(&env.ctx, pds).await.unwrap();

        assert!(matches!(outcomes[0], DishOutcome::Matched(_)));
        assert!(matches!(outcomes[1], DishOutcome::Matched(_)));
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    fn candidate_dish(name: &str, use_count: i64) -> Dish {
        Dish {
            id: uuid::Uuid::now_v7().to_string(),
            brand: "大菜单".into(),
            brand_key: normalize("大菜单"),
            name: name.into(),
            name_key: normalize(name),
            serving_g: None,
            per_serving: RICE,
            source_id: "manual".into(),
            source_kind: crate::model::SourceKind::Manual,
            source_name: "手动录入".into(),
            use_count,
        }
    }

    #[test]
    fn candidate_ranking_cuts_to_200_and_keeps_a_close_match() {
        let mut candidates: Vec<Dish> = (0..250)
            .map(|i| candidate_dish(&format!("菜品{i:04}"), 0))
            .collect();
        candidates.push(candidate_dish("牛めし（並盛）", 0));

        let ranked = rank_candidates(candidates, "牛めし 並盛");

        assert_eq!(ranked.len(), 200);
        assert!(ranked.iter().any(|d| d.name == "牛めし（並盛）"));
    }

    #[test]
    fn candidate_ranking_keeps_all_when_at_or_under_the_limit() {
        let candidates: Vec<Dish> = (0..200)
            .map(|i| candidate_dish(&format!("菜品{i:04}"), 0))
            .collect();
        assert_eq!(rank_candidates(candidates, "菜品0000").len(), 200);
    }

    #[tokio::test]
    async fn grams_stated_without_serving_g_falls_back_to_components() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, None)
            .await
            .unwrap();
        let pd = dish_with("牛めし（並盛）", Some("松屋"), None, 1.0, Some(400.0));

        let outcomes = resolve_dishes(&env.ctx, vec![pd.clone()]).await.unwrap();

        assert_eq!(outcomes, vec![DishOutcome::Unmatched(pd)]);
        // An exact hit that can't convert grams must never also try a pick call.
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn grams_stated_with_serving_g_converts_to_servings() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, Some(350.0))
            .await
            .unwrap();
        let pd = dish_with("牛めし（並盛）", Some("松屋"), None, 1.0, Some(700.0));

        let outcomes = resolve_dishes(&env.ctx, vec![pd]).await.unwrap();

        let DishOutcome::Matched(m) = &outcomes[0] else {
            panic!("expected a match");
        };
        assert_eq!(m.servings, 2.0);
        assert_eq!(m.grams, Some(700.0));
    }

    #[tokio::test]
    async fn servings_derived_from_grams_are_rounded_for_display() {
        let env = env().await;
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, Some(350.0))
            .await
            .unwrap();
        let pd = dish_with("牛めし（並盛）", Some("松屋"), None, 1.0, Some(400.0));

        let outcomes = resolve_dishes(&env.ctx, vec![pd]).await.unwrap();

        let DishOutcome::Matched(m) = &outcomes[0] else {
            panic!("expected a match");
        };
        // 400 / 350 = 1.142857…
        assert_eq!(m.servings, 1.14);
        assert_eq!(m.grams, Some(400.0));
        assert_eq!(round_servings(0.001), 0.01);
    }

    #[tokio::test]
    async fn priority_manual_beats_dataset_for_dish_matching() {
        let env = env().await;
        let dataset_id = uuid::Uuid::now_v7().to_string();
        let now = env.clock.now().to_rfc3339();
        sqlx::query(
            "INSERT INTO diet_food_sources (id, kind, name, priority, enabled, origin_json, created_at, updated_at)
             VALUES ($1, 'dataset', '数据集', 5, 1, '{}', $2, $3)",
        )
        .bind(&dataset_id)
        .bind(&now)
        .bind(&now)
        .execute(&env.ctx.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO diet_dishes
                (id, source_id, brand, brand_key, name, name_key, serving_g, kcal, protein_g,
                 carbs_g, fat_g, fiber_g, use_count, created_at, updated_at)
             VALUES ($1, $2, '松屋', '松屋', '牛めし（並盛）', $3, 350.0, 1.0, 1.0, 1.0, 1.0, 0, 0, $4, $5)",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(&dataset_id)
        .bind(normalize("牛めし（並盛）"))
        .bind(&now)
        .bind(&now)
        .execute(&env.ctx.db)
        .await
        .unwrap();
        upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), RICE, Some(350.0))
            .await
            .unwrap();
        let pd = dish_with("牛めし（並盛）", Some("松屋"), None, 1.0, None);

        let outcomes = resolve_dishes(&env.ctx, vec![pd]).await.unwrap();

        let DishOutcome::Matched(m) = &outcomes[0] else {
            panic!("expected a match");
        };
        assert_eq!(m.dish.per_serving, RICE);
    }

    #[test]
    fn dice_score_favors_close_matches() {
        assert_eq!(dice_score("牛めし並盛", "牛めし並盛"), 1.0);
        assert!(dice_score("牛めし並盛", "牛めし大盛") > dice_score("牛めし並盛", "味噌汁"));
        assert_eq!(dice_score("a", "a"), 1.0);
        assert_eq!(dice_score("a", "b"), 0.0);
    }
}
