//! `FoodResolver`: the local food table first, a parser-model estimate on a miss, per
//! `docs/PHASE1_SPEC.md` §8.2/§8.3.

use std::collections::HashSet;
use std::sync::Arc;

use lifeagent_core::plugin::{PluginContext, PluginError};
use lifeagent_llm::json::complete_json;
use lifeagent_llm::types::{Message, Role};
use serde_json::json;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::foods::{check_nutrition, normalize};
use crate::model::Macros;
use crate::prompts::FOOD_ESTIMATE;
use crate::sources::{insert_estimate, Food, SourceChain};

/// At most this many food estimates in flight at once (§8.2).
const MAX_CONCURRENT_ESTIMATES: usize = 4;

/// Shown to the user when a food's nutrition can't be estimated even after one re-ask (§8.3).
pub const ESTIMATE_FAILED: &str = "无法估算该食物营养";

pub async fn resolve(ctx: &PluginContext, name: &str) -> Result<Food, PluginError> {
    let mut foods = resolve_many(ctx, &[name.to_string()]).await?;
    foods
        .pop()
        .ok_or_else(|| PluginError::Other("resolve_many returned no food".into()))
}

/// Resolves every name to a food, in input order, counting one use per name. Misses are
/// estimated concurrently (at most 4 in flight) and inserted in input order; a name repeated
/// in `names` is estimated once.
pub async fn resolve_many(ctx: &PluginContext, names: &[String]) -> Result<Vec<Food>, PluginError> {
    let chain = SourceChain::load(ctx).await?;

    let mut misses = Vec::new();
    let mut seen_keys = HashSet::new();
    for name in names {
        let key = normalize(name);
        if key.is_empty() {
            return Err(PluginError::InvalidArgs(
                "food name must not be empty".into(),
            ));
        }
        if chain.lookup(name).await?.is_none() && seen_keys.insert(key) {
            misses.push(name.trim().to_string());
        }
    }

    let estimates = estimate_all(ctx, &misses).await?;
    for (name, per_100g) in misses.iter().zip(estimates) {
        insert_estimate(ctx, name, per_100g).await?;
    }

    let mut foods = Vec::with_capacity(names.len());
    for name in names {
        let food = chain
            .lookup(name)
            .await?
            .ok_or_else(|| PluginError::Other(format!("food {name:?} missing after insert")))?;
        chain.record_use(&food).await?;
        foods.push(food);
    }
    Ok(foods)
}

async fn estimate_all(ctx: &PluginContext, names: &[String]) -> Result<Vec<Macros>, PluginError> {
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_ESTIMATES));
    let mut tasks = JoinSet::new();
    for (i, name) in names.iter().enumerate() {
        let ctx = ctx.clone();
        let name = name.clone();
        let permits = permits.clone();
        tasks.spawn(async move {
            let _permit = permits
                .acquire_owned()
                .await
                .map_err(|e| PluginError::Other(e.to_string()))?;
            estimate(&ctx, &name).await.map(|m| (i, m))
        });
    }

    let mut results = vec![None; names.len()];
    while let Some(joined) = tasks.join_next().await {
        let (i, macros) =
            joined.map_err(|e| PluginError::Other(format!("estimate task failed: {e}")))??;
        results[i] = Some(macros);
    }
    results
        .into_iter()
        .map(|m| m.ok_or_else(|| PluginError::Other("missing estimate".into())))
        .collect()
}

/// Asks the parser model for per-100 g nutrition, with one re-ask if the values fail the
/// §8.3 validity rules.
async fn estimate(ctx: &PluginContext, name: &str) -> Result<Macros, PluginError> {
    let mut messages = vec![Message::User {
        content: FOOD_ESTIMATE.replace("{name}", name),
    }];
    let first: Macros = complete_json(&ctx.llm, Role::Parser, messages.clone()).await?;
    let problem = match check_nutrition(&first) {
        Ok(checked) => return Ok(checked),
        Err(problem) => problem,
    };

    messages.push(Message::Assistant {
        content: Some(macros_json_string(&first)),
        tool_calls: vec![],
    });
    messages.push(Message::User {
        content: format!(
            "Those values are not valid: {problem}. Return corrected JSON in the same format."
        ),
    });
    let second: Macros = complete_json(&ctx.llm, Role::Parser, messages).await?;
    check_nutrition(&second).map_err(|problem| {
        tracing::debug!(
            food = name,
            problem,
            "food estimate still invalid after re-ask"
        );
        PluginError::Other(ESTIMATE_FAILED.into())
    })
}

fn macros_json_string(m: &Macros) -> String {
    json!({
        "kcal": m.kcal,
        "protein_g": m.protein_g,
        "carbs_g": m.carbs_g,
        "fat_g": m.fat_g,
        "fiber_g": m.fiber_g,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{add_user_alias, upsert_manual_food};
    use crate::testutil::{env, estimate_responder, json_response};
    use serde_json::json;

    #[tokio::test]
    async fn hit_by_name_makes_no_llm_request() {
        let env = env().await;
        upsert_manual_food(&env.ctx, "熟米饭", crate::testutil::RICE)
            .await
            .unwrap();

        let food = resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(food.name, "熟米饭");
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn hit_by_alias_returns_the_aliased_food() {
        let env = env().await;
        let egg = upsert_manual_food(&env.ctx, "卤鸡蛋", crate::testutil::EGG)
            .await
            .unwrap();
        add_user_alias(&env.ctx, "卤鸡蛋", "卤蛋").await.unwrap();

        assert_eq!(resolve(&env.ctx, "卤蛋").await.unwrap().id, egg.id);
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn miss_is_estimated_then_cached() {
        let env = env().await;
        env.mock.push_closure(estimate_responder);

        let first = resolve(&env.ctx, "熟米饭").await.unwrap();
        let second = resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(first.source_kind, crate::model::SourceKind::LlmEstimate);
        assert_eq!(first.per_100g.kcal, 116.0);
        assert_eq!(second.id, first.id);
        // The second call hit the table: only one LLM request in total.
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn resolve_many_keeps_order_estimates_duplicates_once_and_counts_uses() {
        let env = env().await;
        env.mock.push_closure(estimate_responder);
        env.mock.push_closure(estimate_responder);
        let names: Vec<String> = ["熟米饭", "鸡蛋", "熟米饭"].map(String::from).to_vec();

        let foods = resolve_many(&env.ctx, &names).await.unwrap();

        let got: Vec<&str> = foods.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(got, vec!["熟米饭", "鸡蛋", "熟米饭"]);
        assert_eq!(env.mock.received_requests().len(), 2);
        let rice = SourceChain::load(&env.ctx)
            .await
            .unwrap()
            .lookup("熟米饭")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rice.use_count, 2);
    }

    #[tokio::test]
    async fn invalid_estimate_is_re_asked_once() {
        let env = env().await;
        // fiber > carbs: invalid.
        env.mock.push_response(json_response(json!({
            "kcal": 50, "protein_g": 1, "carbs_g": 5, "fat_g": 0, "fiber_g": 10
        })));
        env.mock.push_response(json_response(json!({
            "kcal": 116, "protein_g": 2.6, "carbs_g": 25.9, "fat_g": 0.3, "fiber_g": 0.3
        })));

        let food = resolve(&env.ctx, "熟米饭").await.unwrap();

        assert_eq!(food.per_100g.kcal, 116.0);
        let requests = env.mock.received_requests();
        assert_eq!(requests.len(), 2);
        // The re-ask carries the model's bad output and the rule that failed.
        let last = requests[1].messages.last().unwrap();
        assert!(matches!(last, Message::User { content } if content.contains("fiber_g")));
    }

    #[tokio::test]
    async fn still_invalid_after_re_ask_fails_and_stores_nothing() {
        let env = env().await;
        for _ in 0..2 {
            env.mock.push_response(json_response(json!({
                "kcal": 50, "protein_g": 1, "carbs_g": 5, "fat_g": 0, "fiber_g": 10
            })));
        }

        let err = resolve(&env.ctx, "怪东西").await.unwrap_err();

        assert!(matches!(err, PluginError::Other(ref m) if m == ESTIMATE_FAILED));
        assert_eq!(
            SourceChain::load(&env.ctx)
                .await
                .unwrap()
                .lookup("怪东西")
                .await
                .unwrap(),
            None
        );
    }
}
