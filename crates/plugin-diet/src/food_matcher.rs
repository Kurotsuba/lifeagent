//! Matching a food name against the imported data, per `docs/FOOD_MATCHING.md`: candidates by
//! character-bigram similarity, one model call to pick the same food (or none), and the
//! bookkeeping that makes each name cost that at most once.
//!
//! `resolver::resolve_many` calls [`match_pending`] for every name whose only hit is a cached
//! estimate, or that has no hit at all, before it estimates anything.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use lifeagent_core::plugin::{PluginContext, PluginError};
use lifeagent_llm::json::complete_json_once;
use lifeagent_llm::types::{Message, Role};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::foods::{bigrams, dice_score, normalize};
use crate::prompts::FOOD_MATCH;
use crate::resolver::FoodQuery;
use crate::sources::{Food, SourceChain};

/// §4: candidates sent to the model per name.
pub const MAX_CANDIDATES: usize = 8;

/// A food that could be picked: the winning row for one `name_key` across enabled `manual` and
/// `dataset` sources.
#[derive(Debug, Clone)]
pub(crate) struct PoolRow {
    pub name: String,
    pub name_key: String,
    pub use_count: i64,
    /// Strings a query is compared with: the name key, then the winner's own source aliases.
    strings: Vec<String>,
}

/// §4's pool with its in-memory index.
pub(crate) struct Pool {
    rows: Vec<PoolRow>,
    /// Bigram → rows. A row string under 2 characters is indexed as `(c, '\0')`.
    bigram_index: HashMap<(char, char), Vec<usize>>,
    /// Character → rows, only for queries under 2 characters (`梨` finds `梨(平均)`).
    char_index: HashMap<char, Vec<usize>>,
}

impl Pool {
    pub(crate) fn from_rows(rows: Vec<PoolRow>) -> Self {
        let mut bigram_index: HashMap<(char, char), Vec<usize>> = HashMap::new();
        let mut char_index: HashMap<char, Vec<usize>> = HashMap::new();
        for (i, row) in rows.iter().enumerate() {
            for s in &row.strings {
                for token in tokens(s) {
                    push_unique(bigram_index.entry(token).or_default(), i);
                }
                for c in s.chars() {
                    push_unique(char_index.entry(c).or_default(), i);
                }
            }
        }
        Pool {
            rows,
            bigram_index,
            char_index,
        }
    }

    /// The winning `manual`/`dataset` row per name key, with source aliases (§4).
    pub(crate) async fn load(ctx: &PluginContext) -> Result<Self, PluginError> {
        let foods: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT f.id, f.name, f.name_key, f.use_count, s.priority
             FROM diet_foods f JOIN diet_food_sources s ON s.id = f.source_id
             WHERE s.enabled = 1 AND s.kind IN ('manual', 'dataset')",
        )
        .fetch_all(&ctx.db)
        .await?;

        // Lowest priority number wins per key, as in `SourceChain::walk`.
        let mut winners: HashMap<String, (i64, String, String, i64)> = HashMap::new();
        for (id, name, name_key, use_count, priority) in foods {
            match winners.get(&name_key) {
                Some((p, ..)) if *p <= priority => {}
                _ => {
                    winners.insert(name_key, (priority, id, name, use_count));
                }
            }
        }

        let mut by_food_id: HashMap<String, usize> = HashMap::with_capacity(winners.len());
        let mut rows = Vec::with_capacity(winners.len());
        for (name_key, (_, id, name, use_count)) in winners {
            by_food_id.insert(id, rows.len());
            rows.push(PoolRow {
                name,
                strings: vec![name_key.clone()],
                name_key,
                use_count,
            });
        }

        let aliases: Vec<(String, String)> = sqlx::query_as(
            "SELECT a.food_id, a.alias_key
             FROM diet_food_aliases a JOIN diet_food_sources s ON s.id = a.source_id
             WHERE s.enabled = 1 AND s.kind IN ('manual', 'dataset')",
        )
        .fetch_all(&ctx.db)
        .await?;
        for (food_id, alias_key) in aliases {
            if let Some(&i) = by_food_id.get(&food_id) {
                rows[i].strings.push(alias_key);
            }
        }
        // `HashMap` order is arbitrary; keep the pool (and so ties) deterministic.
        rows.sort_by(|a, b| a.name_key.cmp(&b.name_key));
        Ok(Pool::from_rows(rows))
    }

    /// §4 ranking: the top [`MAX_CANDIDATES`] rows sharing a token with any query string, best
    /// score first, ties broken by `use_count` descending and then name.
    pub(crate) fn candidates(&self, query: &FoodQuery) -> Vec<&PoolRow> {
        let queries = query_keys(query);
        let mut hits: HashSet<usize> = HashSet::new();
        for q in &queries {
            if q.chars().count() < 2 {
                if let Some(c) = q.chars().next() {
                    hits.extend(self.char_index.get(&c).into_iter().flatten());
                }
            } else {
                for token in tokens(q) {
                    hits.extend(self.bigram_index.get(&token).into_iter().flatten());
                }
            }
        }

        let mut scored: Vec<(f64, &PoolRow)> = hits
            .into_iter()
            .map(|i| {
                let row = &self.rows[i];
                let best = row
                    .strings
                    .iter()
                    .flat_map(|s| queries.iter().map(move |q| similarity(s, q)))
                    .fold(0.0, f64::max);
                (best, row)
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then(b.1.use_count.cmp(&a.1.use_count))
                .then_with(|| a.1.name.cmp(&b.1.name))
                .then_with(|| a.1.name_key.cmp(&b.1.name_key))
        });
        scored
            .into_iter()
            .take(MAX_CANDIDATES)
            .map(|(_, row)| row)
            .collect()
    }
}

fn push_unique(v: &mut Vec<usize>, i: usize) {
    if v.last() != Some(&i) {
        v.push(i);
    }
}

/// Index tokens of one string: its bigrams, or `(c, '\0')` for a 1-character string.
fn tokens(s: &str) -> Vec<(char, char)> {
    let b = bigrams(s);
    if !b.is_empty() {
        return b;
    }
    s.chars().next().map(|c| (c, '\0')).into_iter().collect()
}

/// The normalized name and alternate names of a query, without blanks or duplicates.
fn query_keys(query: &FoodQuery) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in std::iter::once(&query.name).chain(&query.alt_names) {
        let key = normalize(s);
        if !key.is_empty() && !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// Bigram Dice, except that a string under 2 characters (which has no bigrams) is compared by
/// characters, so `梨` scores against `梨(平均)` instead of always 0.
fn similarity(a: &str, b: &str) -> f64 {
    let (la, lb) = (a.chars().count(), b.chars().count());
    if la >= 2 && lb >= 2 {
        return dice_score(a, b);
    }
    if a == b {
        return 1.0;
    }
    let mut pool: Vec<char> = b.chars().collect();
    let mut shared = 0usize;
    for c in a.chars() {
        if let Some(pos) = pool.iter().position(|x| *x == c) {
            pool.remove(pos);
            shared += 1;
        }
    }
    if la + lb == 0 {
        return 0.0;
    }
    (2 * shared) as f64 / (la + lb) as f64
}

/// Matches every query in `pending`, which must have distinct normalized names and each be a
/// miss or a cached-estimate hit (§6 steps 3–5). Runs at most one model call. Returns the food
/// each picked name resolves to; a pick is also stored as an alias, so later lookups redirect.
///
/// A failed model call is not an error: the names are left as they were, with nothing recorded,
/// so the next meal tries again.
pub(crate) async fn match_pending(
    ctx: &PluginContext,
    chain: &SourceChain,
    pending: &[&FoodQuery],
) -> Result<HashMap<String, Food>, PluginError> {
    let mut picked = HashMap::new();
    let latest_dataset_change = latest_dataset_update(ctx).await?;

    let mut to_match: Vec<&FoodQuery> = Vec::new();
    for query in pending {
        let key = normalize(&query.name);
        if key.is_empty() || has_user_alias(ctx, &key).await? {
            continue;
        }
        if check_is_fresh(ctx, &key, latest_dataset_change).await? {
            continue;
        }
        to_match.push(query);
    }
    if to_match.is_empty() {
        return Ok(picked);
    }

    let pool = Pool::load(ctx).await?;
    let mut targets: Vec<(&FoodQuery, Vec<&PoolRow>)> = Vec::new();
    for query in to_match {
        let candidates = pool.candidates(query);
        if candidates.is_empty() {
            record_check(ctx, &normalize(&query.name)).await?;
        } else {
            targets.push((query, candidates));
        }
    }
    if targets.is_empty() {
        return Ok(picked);
    }

    // A failed or wrong-shaped answer just means "estimate as before", never a failed meal. The
    // error text can quote model output, so it stays at DEBUG (same rule as the dish pick).
    let answers = match run_match(ctx, &targets).await {
        Ok(answers) => answers,
        Err(e) => {
            tracing::warn!("food match failed; falling back to estimates");
            tracing::debug!(error = %e, "food match error");
            return Ok(picked);
        }
    };

    for ((query, candidates), answer) in targets.iter().zip(answers) {
        let key = normalize(&query.name);
        let Some(row) = answer.and_then(|i| candidates.get(i)) else {
            record_check(ctx, &key).await?;
            continue;
        };
        let Some(food) = chain.lookup_direct(&row.name_key).await? else {
            // The row vanished between the pool load and now; treat it like a null.
            continue;
        };
        store_pick(ctx, &key, &row.name_key).await?;
        picked.insert(key, food);
    }
    Ok(picked)
}

#[derive(Deserialize)]
struct RawMatches {
    // Required: an answer without it is wrong-shaped, i.e. a failed call (nothing recorded),
    // not a valid "no match" for every name.
    matches: Vec<RawMatch>,
}

#[derive(Deserialize)]
struct RawMatch {
    food: usize,
    #[serde(default)]
    candidate: Option<usize>,
}

/// The one `FOOD_MATCH` call (§5). Returns, per target, the chosen candidate index or `None`.
/// An out-of-range index, or a food missing from the output, counts as `None`.
async fn run_match(
    ctx: &PluginContext,
    targets: &[(&FoodQuery, Vec<&PoolRow>)],
) -> Result<Vec<Option<usize>>, PluginError> {
    let entries: Vec<Value> = targets
        .iter()
        .enumerate()
        .map(|(i, (query, candidates))| {
            json!({
                "food": i,
                "name": query.name,
                "candidates": candidates
                    .iter()
                    .enumerate()
                    .map(|(j, row)| json!({"index": j, "name": row.name}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    // `alt_names` are deliberately not sent: they only widen the search (§3).
    let pending_json =
        serde_json::to_string(&entries).map_err(|e| PluginError::Other(e.to_string()))?;
    let prompt = FOOD_MATCH.replace("{pending_foods}", &pending_json);

    let raw: RawMatches = complete_json_once(
        &ctx.llm,
        Role::Parser,
        vec![Message::User { content: prompt }],
    )
    .await?;

    let mut result = vec![None; targets.len()];
    for m in raw.matches {
        let Some(candidates) = targets.get(m.food).map(|(_, c)| c) else {
            continue;
        };
        result[m.food] = m.candidate.filter(|&c| c < candidates.len());
    }
    Ok(result)
}

/// The newest `updated_at` over `dataset` sources: import, replace, enable, disable and reorder
/// all bump it (§6.1).
async fn latest_dataset_update(ctx: &PluginContext) -> Result<Option<DateTime<Utc>>, PluginError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT updated_at FROM diet_food_sources WHERE kind = 'dataset'")
            .fetch_all(&ctx.db)
            .await?;
    // An unparsable stamp can't be compared; leave it out rather than fail the meal.
    Ok(rows
        .iter()
        .filter_map(|(s,)| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .max())
}

/// §6.1: a check is fresh unless a dataset changed after it was made.
async fn check_is_fresh(
    ctx: &PluginContext,
    key: &str,
    latest_dataset_change: Option<DateTime<Utc>>,
) -> Result<bool, PluginError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT checked_at FROM diet_food_match_checks WHERE name_key = ?")
            .bind(key)
            .fetch_optional(&ctx.db)
            .await?;
    let Some((checked_at,)) = row else {
        return Ok(false);
    };
    let Ok(checked_at) = DateTime::parse_from_rfc3339(&checked_at) else {
        return Ok(false);
    };
    Ok(match latest_dataset_change {
        Some(changed) => changed <= checked_at.with_timezone(&Utc),
        None => true,
    })
}

/// Upserts `checked_at = now` for `key` (§6.1).
pub(crate) async fn record_check(ctx: &PluginContext, key: &str) -> Result<(), PluginError> {
    sqlx::query(
        "INSERT INTO diet_food_match_checks (name_key, checked_at) VALUES (?, ?)
         ON CONFLICT(name_key) DO UPDATE SET checked_at = excluded.checked_at",
    )
    .bind(key)
    .bind(ctx.clock.now().to_rfc3339())
    .execute(&ctx.db)
    .await?;
    Ok(())
}

/// An explicit user redirect is the user's decision; matching never overrides it.
async fn has_user_alias(ctx: &PluginContext, key: &str) -> Result<bool, PluginError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT origin FROM diet_name_aliases WHERE alias_key = ?")
            .bind(key)
            .fetch_optional(&ctx.db)
            .await?;
    Ok(matches!(row, Some((origin,)) if origin == "user"))
}

/// §6.2: stores `key → target_key` with `origin = 'match'`, keeping the one-hop rule. Returns
/// whether anything was stored.
async fn store_pick(ctx: &PluginContext, key: &str, target_key: &str) -> Result<bool, PluginError> {
    if key == target_key {
        return Ok(false);
    }
    let mut tx = ctx.db.begin().await?;
    let target_is_alias: Option<(String,)> =
        sqlx::query_as("SELECT alias_key FROM diet_name_aliases WHERE alias_key = ?")
            .bind(target_key)
            .fetch_optional(&mut *tx)
            .await?;
    if target_is_alias.is_some() {
        return Ok(false);
    }
    // Aliases that pointed at `key` now point at the picked row, so no chain forms. One that
    // would end up pointing at itself is dropped.
    sqlx::query("UPDATE diet_name_aliases SET target_key = ? WHERE target_key = ?")
        .bind(target_key)
        .bind(key)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM diet_name_aliases WHERE alias_key = target_key")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO diet_name_aliases (alias_key, target_key, created_at, origin)
         VALUES (?, ?, ?, 'match')
         ON CONFLICT(alias_key) DO UPDATE SET
            target_key = excluded.target_key, created_at = excluded.created_at,
            origin = excluded.origin",
    )
    .bind(key)
    .bind(target_key)
    .bind(ctx.clock.now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Macros, SourceKind};
    use crate::resolver::{resolve, resolve_many};
    use crate::sources::{add_user_alias, insert_estimate, upsert_manual_food};
    use crate::testutil::{add_dataset, env, estimate_responder, json_response, touch_dataset};
    use crate::testutil::{EGG, RICE};
    use lifeagent_llm::error::LlmError;

    fn pick(food: usize, candidate: Option<usize>) -> lifeagent_llm::types::ChatResponse {
        json_response(json!({"matches": [{"food": food, "candidate": candidate}]}))
    }

    async fn alias_row(
        env: &lifeagent_core::test_support::TestEnv,
        alias: &str,
    ) -> Option<(String, String)> {
        sqlx::query_as("SELECT target_key, origin FROM diet_name_aliases WHERE alias_key = ?")
            .bind(alias)
            .fetch_optional(&env.ctx.db)
            .await
            .unwrap()
    }

    async fn check_count(env: &lifeagent_core::test_support::TestEnv) -> i64 {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM diet_food_match_checks")
            .fetch_one(&env.ctx.db)
            .await
            .unwrap();
        n
    }

    fn names<'a>(rows: &[&'a PoolRow]) -> Vec<&'a str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[tokio::test]
    async fn a_close_name_ranks_above_unrelated_rows() {
        let env = env().await;
        add_dataset(
            &env,
            "表",
            &[("苹果", RICE), ("稻米饭(蒸)", RICE), ("牛奶", EGG)],
        )
        .await;
        let pool = Pool::load(&env.ctx).await.unwrap();

        let got = pool.candidates(&FoodQuery::new("熟米饭"));

        assert_eq!(names(&got), vec!["稻米饭(蒸)"]);
    }

    #[tokio::test]
    async fn a_one_character_name_finds_longer_rows() {
        let env = env().await;
        add_dataset(&env, "表", &[("梨(平均)", RICE), ("苹果", RICE)]).await;
        let pool = Pool::load(&env.ctx).await.unwrap();

        let got = pool.candidates(&FoodQuery::new("梨"));

        assert_eq!(names(&got), vec!["梨(平均)"]);
    }

    #[tokio::test]
    async fn a_name_sharing_no_bigram_has_no_candidates_and_makes_no_call() {
        let env = env().await;
        add_dataset(&env, "表", &[("苹果", RICE)]).await;
        env.mock.push_closure(estimate_responder);

        let food = resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(food.source_kind, SourceKind::LlmEstimate);
        // Only the estimate; no match call.
        assert_eq!(env.mock.received_requests().len(), 1);
        assert_eq!(check_count(&env).await, 1);
    }

    #[tokio::test]
    async fn estimate_rows_and_disabled_datasets_are_never_candidates() {
        let env = env().await;
        insert_estimate(&env.ctx, "稻米饭(估)", RICE).await.unwrap();
        let off = add_dataset(&env, "关", &[("稻米饭(关)", RICE)]).await;
        sqlx::query("UPDATE diet_food_sources SET enabled = 0 WHERE id = ?")
            .bind(&off)
            .execute(&env.ctx.db)
            .await
            .unwrap();
        add_dataset(&env, "开", &[("稻米饭(开)", RICE)]).await;
        let pool = Pool::load(&env.ctx).await.unwrap();

        let got = pool.candidates(&FoodQuery::new("熟米饭"));

        assert_eq!(names(&got), vec!["稻米饭(开)"]);
    }

    #[tokio::test]
    async fn only_the_top_eight_are_kept_and_ties_go_to_use_count() {
        let env = env().await;
        let foods: Vec<(String, Macros)> = (0..12).map(|i| (format!("米饭{i:02}"), RICE)).collect();
        let refs: Vec<(&str, Macros)> = foods.iter().map(|(n, m)| (n.as_str(), *m)).collect();
        add_dataset(&env, "表", &refs).await;
        sqlx::query("UPDATE diet_foods SET use_count = 5 WHERE name_key = '米饭11'")
            .execute(&env.ctx.db)
            .await
            .unwrap();
        let pool = Pool::load(&env.ctx).await.unwrap();

        let got = pool.candidates(&FoodQuery::new("米饭"));

        assert_eq!(got.len(), MAX_CANDIDATES);
        assert_eq!(got[0].name, "米饭11");
    }

    #[tokio::test]
    async fn a_source_alias_is_searched_and_the_winner_row_is_returned() {
        let env = env().await;
        let id = add_dataset(&env, "表", &[("马铃薯(土豆)", RICE)]).await;
        let (food_id,): (String,) = sqlx::query_as("SELECT id FROM diet_foods WHERE source_id = ?")
            .bind(&id)
            .fetch_one(&env.ctx.db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO diet_food_aliases (source_id, alias_key, food_id) VALUES (?, '洋芋', ?)",
        )
        .bind(&id)
        .bind(&food_id)
        .execute(&env.ctx.db)
        .await
        .unwrap();
        let pool = Pool::load(&env.ctx).await.unwrap();

        let got = pool.candidates(&FoodQuery::new("洋芋"));

        assert_eq!(names(&got), vec!["马铃薯(土豆)"]);
    }

    #[tokio::test]
    async fn a_pick_logs_the_dataset_row_stores_an_alias_and_is_not_asked_again() {
        let env = env().await;
        add_dataset(&env, "表", &[("稻米饭(蒸)", RICE)]).await;
        env.mock.push_response(pick(0, Some(0)));

        let food = resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(food.name, "稻米饭(蒸)");
        assert_eq!(food.source_kind, SourceKind::Dataset);
        // No estimate call.
        assert_eq!(env.mock.received_requests().len(), 1);
        assert_eq!(
            alias_row(&env, "熟米饭").await,
            Some(("稻米饭(蒸)".into(), "match".into()))
        );

        let again = resolve(&env.ctx, "熟米饭").await.unwrap();
        assert_eq!(again.id, food.id);
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn the_prompt_states_the_state_rule_and_numbers_candidates() {
        let env = env().await;
        add_dataset(&env, "表", &[("稻米饭(蒸)", RICE), ("米饭(粳米)", RICE)]).await;
        env.mock.push_response(pick(0, Some(0)));

        resolve(&env.ctx, "熟米饭").await.unwrap();

        let requests = env.mock.received_requests();
        let Some(Message::User { content }) = requests[0].messages.first() else {
            panic!("expected a user message");
        };
        assert!(content.contains("same food in the same state"));
        assert!(content.contains(r#""index":0"#) && content.contains(r#""index":1"#));
        assert!(content.contains(r#""food":0"#));
    }

    #[tokio::test]
    async fn a_cached_estimate_does_not_block_matching() {
        let env = env().await;
        insert_estimate(&env.ctx, "熟米饭", EGG).await.unwrap();
        add_dataset(&env, "表", &[("稻米饭(蒸)", RICE)]).await;
        env.mock.push_response(pick(0, Some(0)));

        let food = resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(food.name, "稻米饭(蒸)");
        assert_eq!(food.per_100g, RICE);
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn a_manual_row_is_an_exact_hit_and_never_matched() {
        let env = env().await;
        upsert_manual_food(&env.ctx, "熟米饭", RICE).await.unwrap();
        add_dataset(&env, "表", &[("稻米饭(蒸)", EGG)]).await;

        let food = resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(food.source_kind, SourceKind::Manual);
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn null_records_a_check_and_is_retried_only_after_a_dataset_changes() {
        let env = env().await;
        let dataset = add_dataset(&env, "表", &[("稻米饭(蒸)", RICE)]).await;
        env.mock.push_response(pick(0, None));
        env.mock.push_closure(estimate_responder);

        let first = resolve(&env.ctx, "熟米饭").await.unwrap();
        assert_eq!(first.source_kind, SourceKind::LlmEstimate);
        assert_eq!(env.mock.received_requests().len(), 2);
        assert_eq!(check_count(&env).await, 1);

        // Fresh check: no new call.
        resolve(&env.ctx, "熟米饭").await.unwrap();
        assert_eq!(env.mock.received_requests().len(), 2);

        // A dataset changed after the check: asked again.
        env.clock.advance(chrono::Duration::minutes(5));
        touch_dataset(&env, &dataset).await;
        env.mock.push_response(pick(0, None));
        resolve(&env.ctx, "熟米饭").await.unwrap();
        assert_eq!(env.mock.received_requests().len(), 3);
    }

    #[tokio::test]
    async fn a_wrong_shaped_answer_or_provider_error_estimates_and_records_no_check() {
        let env = env().await;
        add_dataset(&env, "表", &[("稻米饭(蒸)", RICE), ("鸡蛋(煮)", EGG)]).await;
        env.mock
            .push_response(json_response(json!({"answer": "none"})));
        env.mock.push_closure(estimate_responder);
        let first = resolve(&env.ctx, "熟米饭").await.unwrap();
        assert_eq!(first.source_kind, SourceKind::LlmEstimate);
        // One match request (no repair) plus the estimate.
        assert_eq!(env.mock.received_requests().len(), 2);
        assert_eq!(check_count(&env).await, 0);

        env.mock.push_error(LlmError::Config("boom".into()));
        env.mock.push_closure(estimate_responder);
        let second = resolve(&env.ctx, "鸡蛋").await.unwrap();
        assert_eq!(second.source_kind, SourceKind::LlmEstimate);
        assert_eq!(env.mock.received_requests().len(), 4);
        assert_eq!(check_count(&env).await, 0);
    }

    #[tokio::test]
    async fn three_pending_names_share_one_request() {
        let env = env().await;
        add_dataset(
            &env,
            "表",
            &[("稻米饭(蒸)", RICE), ("番茄", RICE), ("鸡蛋(煮)", EGG)],
        )
        .await;
        env.mock.push_response(json_response(json!({"matches": [
            {"food": 0, "candidate": 0},
            {"food": 1, "candidate": 0},
            {"food": 2, "candidate": 0},
        ]})));
        let queries: Vec<FoodQuery> = ["熟米饭", "番茄炒", "鸡蛋"].map(FoodQuery::new).to_vec();

        let foods = resolve_many(&env.ctx, &queries).await.unwrap();

        let got: Vec<&str> = foods.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(got, vec!["稻米饭(蒸)", "番茄", "鸡蛋(煮)"]);
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn a_repeated_name_is_matched_once() {
        let env = env().await;
        add_dataset(&env, "表", &[("稻米饭(蒸)", RICE)]).await;
        env.mock.push_response(pick(0, Some(0)));
        let queries: Vec<FoodQuery> = ["熟米饭", "熟米饭"].map(FoodQuery::new).to_vec();

        let foods = resolve_many(&env.ctx, &queries).await.unwrap();

        assert_eq!(foods[0].id, foods[1].id);
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn an_out_of_range_index_counts_as_null() {
        let env = env().await;
        add_dataset(&env, "表", &[("稻米饭(蒸)", RICE)]).await;
        env.mock.push_response(pick(0, Some(7)));
        env.mock.push_closure(estimate_responder);

        let food = resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(food.source_kind, SourceKind::LlmEstimate);
        assert_eq!(check_count(&env).await, 1);
        assert!(alias_row(&env, "熟米饭").await.is_none());
    }

    #[tokio::test]
    async fn a_pick_repoints_aliases_that_targeted_the_matched_name() {
        let env = env().await;
        insert_estimate(&env.ctx, "熟米饭", EGG).await.unwrap();
        add_user_alias(&env.ctx, "熟米饭", "米饭").await.unwrap();
        add_dataset(&env, "表", &[("稻米饭(蒸)", RICE)]).await;
        env.mock.push_response(pick(0, Some(0)));

        resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(
            alias_row(&env, "米饭").await,
            Some(("稻米饭(蒸)".into(), "user".into()))
        );
        assert_eq!(
            alias_row(&env, "熟米饭").await,
            Some(("稻米饭(蒸)".into(), "match".into()))
        );
        // No chain: nothing targets an alias key.
        let (chains,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM diet_name_aliases a
             JOIN diet_name_aliases b ON b.alias_key = a.target_key",
        )
        .fetch_one(&env.ctx.db)
        .await
        .unwrap();
        assert_eq!(chains, 0);
    }

    #[tokio::test]
    async fn an_explicit_user_alias_is_never_overridden_by_matching() {
        let env = env().await;
        insert_estimate(&env.ctx, "卤鸡蛋", EGG).await.unwrap();
        add_user_alias(&env.ctx, "卤鸡蛋", "卤蛋").await.unwrap();
        add_dataset(&env, "表", &[("鸡蛋(煮)", RICE)]).await;

        let food = resolve(&env.ctx, "卤蛋").await.unwrap();

        assert_eq!(food.name, "卤鸡蛋");
        assert!(env.mock.received_requests().is_empty());
    }

    #[test]
    fn similarity_scores_short_strings_by_characters() {
        assert_eq!(similarity("梨", "梨"), 1.0);
        assert!(similarity("梨", "梨(平均)") > 0.0);
        assert_eq!(similarity("梨", "苹果"), 0.0);
        assert_eq!(similarity("", ""), 1.0);
        assert!(similarity("熟米饭", "稻米饭(蒸)") > similarity("熟米饭", "苹果"));
    }
}
