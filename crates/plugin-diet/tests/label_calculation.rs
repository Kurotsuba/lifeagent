//! A label question is a calculation, not a meal report or a food-table correction.

use lifeagent_core::events::EventSource;
use lifeagent_core::plugin::{apply_migrations, Plugin, PluginError};
use lifeagent_plugin_diet::model::LabelNutrition;
use lifeagent_plugin_diet::{tools, DietPlugin};
use serde_json::json;

#[test]
fn scales_only_the_label_values_the_user_supplied() {
    let label = LabelNutrition {
        protein_g: Some(10.0),
        ..Default::default()
    };
    let portion = label.for_grams(150.0).unwrap();
    assert_eq!(portion.protein_g, Some(15.0));
    assert_eq!(
        serde_json::to_value(portion).unwrap(),
        json!({"protein_g": 15.0})
    );
}

#[test]
fn preserves_explicit_zero_and_scales_every_supported_nutrient() {
    let label = LabelNutrition {
        kcal: Some(200.0),
        protein_g: Some(10.0),
        carbs_g: Some(30.0),
        fat_g: Some(0.0),
        fiber_g: Some(5.0),
    };
    assert_eq!(
        serde_json::to_value(label.for_grams(50.0).unwrap()).unwrap(),
        json!({"kcal": 100.0, "protein_g": 5.0, "carbs_g": 15.0, "fat_g": 0.0, "fiber_g": 2.5})
    );
}

#[test]
fn decimal_label_totals_do_not_fail_due_to_floating_point_roundoff() {
    let label = LabelNutrition {
        protein_g: Some(0.2),
        carbs_g: Some(83.9),
        fat_g: Some(15.9),
        ..Default::default()
    };
    let result = label.for_grams(100.0).unwrap();
    assert_eq!(result, label);
}

#[test]
fn rejects_non_finite_negative_missing_and_impossible_values() {
    assert!(LabelNutrition::default().for_grams(150.0).is_err());
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 101.0] {
        assert!(LabelNutrition {
            protein_g: Some(value),
            ..Default::default()
        }
        .for_grams(150.0)
        .is_err());
    }
    let label = LabelNutrition {
        protein_g: Some(10.0),
        ..Default::default()
    };
    for grams in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(label.for_grams(grams).is_err());
    }
    assert!(LabelNutrition {
        kcal: Some(900.0),
        ..Default::default()
    }
    .for_grams(f64::MAX)
    .is_err());
    assert!(LabelNutrition {
        protein_g: Some(60.0),
        carbs_g: Some(50.0),
        ..Default::default()
    }
    .for_grams(150.0)
    .is_err());
    assert!(LabelNutrition {
        carbs_g: Some(5.0),
        fiber_g: Some(6.0),
        ..Default::default()
    }
    .for_grams(150.0)
    .is_err());
    // A fiber-only label remains useful; missing carbohydrate is not treated as zero.
    assert!(LabelNutrition {
        fiber_g: Some(6.0),
        ..Default::default()
    }
    .for_grams(150.0)
    .is_ok());
}

#[tokio::test]
async fn label_tool_does_not_record_change_food_data_or_call_a_model() {
    let env = lifeagent_core::test_support::env().await;
    let ctx = &env.ctx;
    apply_migrations(&ctx.db, ctx.clock.as_ref(), &DietPlugin.manifest())
        .await
        .unwrap();
    ctx.events
        .append(
            "body.weight_logged",
            ctx.clock.now(),
            json!({"kg": 60.0}),
            EventSource::Ui,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO diet_foods (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g, created_at, updated_at) VALUES ('existing', 'manual', '已有食品', '已有食品', 150, 5, 20, 5, 0, 'before', 'before')")
        .execute(&ctx.db).await.unwrap();
    let snapshot = || async {
        sqlx::query_scalar::<_, String>("SELECT jsonb_build_object('events', (SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM events e), 'foods', (SELECT jsonb_agg(to_jsonb(f) ORDER BY f.id) FROM diet_foods f), 'sources', (SELECT jsonb_agg(to_jsonb(s) ORDER BY s.id) FROM diet_food_sources s))::text")
            .fetch_one(&ctx.db).await.unwrap()
    };
    let before = snapshot().await;
    let args = json!({"basis": "per_100g", "portion_grams": 150, "per_100g": {"protein_g": 10}});
    assert!(tools::approval_preview(ctx, "diet_calculate_label", &args)
        .await
        .unwrap()
        .is_none());
    let result = tools::call(ctx, "diet_calculate_label", args)
        .await
        .unwrap();
    assert_eq!(result.content["portion"]["protein_g"], 15.0);
    assert_eq!(result.content["source"], "user_label");
    assert_eq!(result.content["recorded"], false);
    assert!(result.content["portion"].get("kcal").is_none());
    assert_eq!(result.summary.kind(), "diet_label_calculated");
    assert_eq!(snapshot().await, before);
    assert!(env.mock.received_requests().is_empty());
}

#[tokio::test]
async fn label_tool_rejects_unsupported_units_and_invalid_arguments() {
    let env = lifeagent_core::test_support::env().await;
    for args in [
        json!({"basis": "per_100ml", "portion_grams": 150, "per_100g": {"protein_g": 10}}),
        json!({"basis": "per_serving", "portion_grams": 150, "per_100g": {"protein_g": 10}}),
        json!({"basis": "per_100g", "portion_grams": "150g", "per_100g": {"protein_g": 10}}),
        json!({"basis": "per_100g", "portion_grams": 0, "per_100g": {"protein_g": 10}}),
        json!({"basis": "per_100g", "portion_grams": 150, "per_100g": {"protein_g": -10}}),
        json!({"basis": "per_100g", "portion_grams": 150, "per_100g": {"protein_mg": 10000}}),
        json!({"basis": "per_100g", "portion_grams": 150, "portion_unit": "ml", "per_100g": {"protein_g": 10}}),
        json!({"basis": "per_100g", "portion_grams": 150, "per_100g": {}}),
    ] {
        assert!(
            matches!(
                tools::call(&env.ctx, "diet_calculate_label", args.clone()).await,
                Err(PluginError::InvalidArgs(_))
            ),
            "accepted {args}"
        );
    }
    assert!(env.mock.received_requests().is_empty());
}
