//! `ToolRegistry`: prefix/uniqueness rules, argument validation and dispatch, per
//! `docs/PHASE1_SPEC.md` §7.7.

use std::collections::HashMap;
use std::sync::Arc;

use lifeagent_llm::types::ToolSpec;
use serde_json::Value;

use crate::core_tools;
use crate::error::CoreError;
use crate::plan::PlanService;
use crate::plugin::{Plugin, PluginContext, PluginError, ToolOutput};

enum ToolOwner {
    Core,
    Plugin(Arc<dyn Plugin>),
}

/// Holds every registered tool's spec and owner. Built once at `CoreApp::build` time; tool
/// names are globally unique and must start with `{plugin_id}_` (core tools: `core_`).
pub struct ToolRegistry {
    specs: Vec<ToolSpec>,
    owners: HashMap<String, ToolOwner>,
    /// The app's one plan service, for the plan core tools.
    planner: Arc<PlanService>,
}

impl ToolRegistry {
    pub fn build(
        plugins: &[Arc<dyn Plugin>],
        planner: Arc<PlanService>,
    ) -> Result<Self, CoreError> {
        let mut registry = ToolRegistry {
            specs: Vec::new(),
            owners: HashMap::new(),
            planner,
        };

        for spec in core_tools::specs() {
            registry.register(spec, "core_", ToolOwner::Core)?;
        }

        for plugin in plugins {
            let manifest = plugin.manifest();
            let prefix = format!("{}_", manifest.id);
            for spec in plugin.tools() {
                registry.register(spec, &prefix, ToolOwner::Plugin(plugin.clone()))?;
            }
        }

        Ok(registry)
    }

    fn register(
        &mut self,
        spec: ToolSpec,
        prefix: &str,
        owner: ToolOwner,
    ) -> Result<(), CoreError> {
        if !spec.name.starts_with(prefix) {
            return Err(CoreError::Validation(format!(
                "tool {:?} must start with prefix {prefix:?}",
                spec.name
            )));
        }
        if self.owners.contains_key(&spec.name) {
            return Err(CoreError::Validation(format!(
                "duplicate tool name {:?}",
                spec.name
            )));
        }
        self.owners.insert(spec.name.clone(), owner);
        self.specs.push(spec);
        Ok(())
    }

    /// All registered tool specs, in registration order (core tools first).
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.specs.clone()
    }

    /// Core tools plus the tools of the plugins in `plugins` (the enabled ones), in
    /// registration order.
    pub fn specs_for(&self, plugins: &[Arc<dyn Plugin>]) -> Vec<ToolSpec> {
        self.specs
            .iter()
            .filter(|spec| self.is_available(&spec.name, plugins))
            .cloned()
            .collect()
    }

    /// The enabled tools permitted by this request's access mode. The dispatcher checks
    /// again, since a model can return undeclared calls or replay them from history.
    pub fn specs_for_mode(&self, plugins: &[Arc<dyn Plugin>], read_only: bool) -> Vec<ToolSpec> {
        self.specs_for(plugins)
            .into_iter()
            .filter(|spec| !read_only || self.is_read_only(&spec.name))
            .collect()
    }

    pub fn is_read_only(&self, name: &str) -> bool {
        match self.owners.get(name) {
            Some(ToolOwner::Core) => name == "core_get_plan",
            Some(ToolOwner::Plugin(owner)) => owner.is_read_only_tool(name),
            None => false,
        }
    }

    /// Whether `name` is a core tool or belongs to one of `plugins`. A tool of a disabled
    /// plugin isn't offered to the model; this also refuses a call to one from stale history.
    pub fn is_available(&self, name: &str, plugins: &[Arc<dyn Plugin>]) -> bool {
        match self.owners.get(name) {
            Some(ToolOwner::Core) => true,
            Some(ToolOwner::Plugin(owner)) => {
                let id = owner.manifest().id;
                plugins.iter().any(|p| p.manifest().id == id)
            }
            None => false,
        }
    }

    /// The owner and spec of `name`, after validating `args` against the spec's JSON Schema.
    fn checked(&self, name: &str, args: &Value) -> Result<&ToolOwner, PluginError> {
        let owner = self
            .owners
            .get(name)
            .ok_or_else(|| PluginError::NotFound(format!("unknown tool {name:?}")))?;
        let spec = self
            .specs
            .iter()
            .find(|s| s.name == name)
            .expect("an owner entry always has a matching spec");
        validate_args(&spec.parameters, args).map_err(PluginError::InvalidArgs)?;
        Ok(owner)
    }

    /// `Some(preview)` if this call must wait for the user's approval (`crate::approvals`).
    /// Validates `args` first, so an invalid call is refused right away rather than queued.
    pub async fn approval_preview(
        &self,
        ctx: &PluginContext,
        name: &str,
        args: &Value,
    ) -> Result<Option<Value>, PluginError> {
        match self.checked(name, args)? {
            ToolOwner::Core => core_tools::approval_preview(ctx, name, args).await,
            ToolOwner::Plugin(plugin) => plugin.approval_preview(ctx, name, args).await,
        }
    }

    /// Validates `args` against the tool's JSON Schema, then routes to core or the owning
    /// plugin. Runs the tool whether or not it needs approval: the caller checks
    /// [`ToolRegistry::approval_preview`] first.
    pub async fn call(
        &self,
        ctx: &PluginContext,
        name: &str,
        args: Value,
    ) -> Result<ToolOutput, PluginError> {
        let owner = self.checked(name, &args)?;
        match owner {
            ToolOwner::Core => core_tools::call(ctx, &self.planner, name, args).await,
            ToolOwner::Plugin(plugin) => plugin.call_tool(ctx, name, args).await,
        }
    }
}

/// Hand-written JSON Schema checker (§7.7): required fields and basic types only —
/// `object`, `string`, `number`, `integer`, `boolean`, `array`, `enum`. No full JSON Schema
/// crate is needed for the shapes this app's tool specs actually use.
pub fn validate_args(schema: &Value, value: &Value) -> Result<(), String> {
    validate_node(schema, value, "args")
}

fn validate_node(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array) {
        if !enum_values.iter().any(|v| v == value) {
            return Err(format!("{path}: {value} is not one of the allowed values"));
        }
    }

    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let obj = value
                .as_object()
                .ok_or_else(|| format!("{path}: expected an object"))?;
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for field in required {
                    if let Some(field) = field.as_str() {
                        if !obj.contains_key(field) {
                            return Err(format!("{path}: missing required field {field:?}"));
                        }
                    }
                }
            }
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (key, prop_schema) in properties {
                    if let Some(v) = obj.get(key) {
                        validate_node(prop_schema, v, &format!("{path}.{key}"))?;
                    }
                }
            }
            Ok(())
        }
        Some("string") => {
            if value.is_string() {
                Ok(())
            } else {
                Err(format!("{path}: expected a string"))
            }
        }
        Some("number") => {
            if value.is_number() {
                Ok(())
            } else {
                Err(format!("{path}: expected a number"))
            }
        }
        Some("integer") => {
            if value.is_i64() || value.is_u64() {
                Ok(())
            } else {
                Err(format!("{path}: expected an integer"))
            }
        }
        Some("boolean") => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(format!("{path}: expected a boolean"))
            }
        }
        Some("array") => {
            let arr = value
                .as_array()
                .ok_or_else(|| format!("{path}: expected an array"))?;
            if let Some(items_schema) = schema.get("items") {
                for (i, item) in arr.iter().enumerate() {
                    validate_node(items_schema, item, &format!("{path}[{i}]"))?;
                }
            }
            Ok(())
        }
        // No `type` constraint (or one this checker doesn't model): accept anything.
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    fn planner() -> Arc<PlanService> {
        Arc::new(PlanService::new(vec![]))
    }

    struct GoodPlugin;

    #[async_trait]
    impl Plugin for GoodPlugin {
        fn manifest(&self) -> crate::plugin::PluginManifest {
            crate::plugin::PluginManifest {
                id: "good",
                name: "Good",
                version: "0.1.0",
                event_types: vec![],
                migrations: vec![],
            }
        }

        fn tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "good_do_thing".into(),
                description: "Does a thing.".into(),
                parameters: json!({"type": "object"}),
            }]
        }

        async fn call_tool(
            &self,
            _ctx: &PluginContext,
            _name: &str,
            _args: Value,
        ) -> Result<ToolOutput, PluginError> {
            Ok(ToolOutput {
                content: json!({"ok": true}),
                summary: crate::plugin::ToolSummary::new("good_did_the_thing"),
            })
        }
    }

    struct BadPrefixPlugin;

    #[async_trait]
    impl Plugin for BadPrefixPlugin {
        fn manifest(&self) -> crate::plugin::PluginManifest {
            crate::plugin::PluginManifest {
                id: "bad",
                name: "Bad",
                version: "0.1.0",
                event_types: vec![],
                migrations: vec![],
            }
        }

        fn tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "wrong_prefix_tool".into(),
                description: "Bad prefix.".into(),
                parameters: json!({"type": "object"}),
            }]
        }

        async fn call_tool(
            &self,
            _ctx: &PluginContext,
            _name: &str,
            _args: Value,
        ) -> Result<ToolOutput, PluginError> {
            unreachable!("registration should fail before any call")
        }
    }

    struct DuplicatePlugin;

    #[async_trait]
    impl Plugin for DuplicatePlugin {
        fn manifest(&self) -> crate::plugin::PluginManifest {
            crate::plugin::PluginManifest {
                id: "dup",
                name: "Dup",
                version: "0.1.0",
                event_types: vec![],
                migrations: vec![],
            }
        }

        fn tools(&self) -> Vec<ToolSpec> {
            vec![
                ToolSpec {
                    name: "dup_do_thing".into(),
                    description: "First.".into(),
                    parameters: json!({"type": "object"}),
                },
                ToolSpec {
                    name: "dup_do_thing".into(),
                    description: "Second, same name.".into(),
                    parameters: json!({"type": "object"}),
                },
            ]
        }

        async fn call_tool(
            &self,
            _ctx: &PluginContext,
            _name: &str,
            _args: Value,
        ) -> Result<ToolOutput, PluginError> {
            unreachable!("registration should fail before any call")
        }
    }

    #[test]
    fn build_registers_core_tools_and_plugin_tools() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(GoodPlugin)];
        let registry = ToolRegistry::build(&plugins, planner()).unwrap();
        let specs = registry.specs();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"core_add_preference"));
        assert!(names.contains(&"good_do_thing"));
    }

    #[test]
    fn read_only_tools_are_explicit_and_unknown_tools_fail_closed() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(GoodPlugin)];
        let registry = ToolRegistry::build(&plugins, planner()).unwrap();
        let names: Vec<_> = registry
            .specs_for_mode(&plugins, true)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["core_get_plan"]);
        assert!(!registry.is_read_only("good_do_thing"));
        assert!(!registry.is_read_only("good_unknown"));
        assert!(!registry.is_read_only("core_unknown"));
        assert!(!registry.is_read_only("core_log_weight"));
        assert!(registry
            .specs_for_mode(&plugins, false)
            .iter()
            .any(|s| s.name == "core_log_weight"));
    }

    #[test]
    fn build_rejects_a_tool_not_matching_the_plugin_prefix() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(BadPrefixPlugin)];
        assert!(matches!(
            ToolRegistry::build(&plugins, planner()),
            Err(CoreError::Validation(_))
        ));
    }

    #[test]
    fn build_rejects_duplicate_tool_names_within_a_plugin() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(DuplicatePlugin)];
        assert!(matches!(
            ToolRegistry::build(&plugins, planner()),
            Err(CoreError::Validation(_))
        ));
    }

    #[test]
    fn build_rejects_a_plugin_tool_colliding_with_a_core_tool_name() {
        struct CollidingPlugin;
        #[async_trait]
        impl Plugin for CollidingPlugin {
            fn manifest(&self) -> crate::plugin::PluginManifest {
                crate::plugin::PluginManifest {
                    id: "core",
                    name: "Fake Core",
                    version: "0.1.0",
                    event_types: vec![],
                    migrations: vec![],
                }
            }
            fn tools(&self) -> Vec<ToolSpec> {
                vec![ToolSpec {
                    name: "core_add_preference".into(),
                    description: "Colliding.".into(),
                    parameters: json!({"type": "object"}),
                }]
            }
            async fn call_tool(
                &self,
                _ctx: &PluginContext,
                _name: &str,
                _args: Value,
            ) -> Result<ToolOutput, PluginError> {
                unreachable!()
            }
        }

        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(CollidingPlugin)];
        assert!(matches!(
            ToolRegistry::build(&plugins, planner()),
            Err(CoreError::Validation(_))
        ));
    }

    #[tokio::test]
    async fn call_routes_to_the_owning_plugin() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(GoodPlugin)];
        let registry = ToolRegistry::build(&plugins, planner()).unwrap();
        let (ctx, _mock, _dir) = crate::test_support::context().await;

        let out = registry
            .call(&ctx, "good_do_thing", json!({}))
            .await
            .unwrap();
        assert_eq!(out.summary.kind(), "good_did_the_thing");
    }

    #[tokio::test]
    async fn call_routes_core_tools_to_core() {
        let registry = ToolRegistry::build(&[], planner()).unwrap();
        let (ctx, _mock, _dir) = crate::test_support::context().await;

        let out = registry
            .call(
                &ctx,
                "core_add_preference",
                json!({"kind": "dislike", "text": "香菜"}),
            )
            .await
            .unwrap();
        assert!(out.content.get("preference_id").is_some());
    }

    #[tokio::test]
    async fn call_of_unknown_tool_is_not_found() {
        let registry = ToolRegistry::build(&[], planner()).unwrap();
        let (ctx, _mock, _dir) = crate::test_support::context().await;

        assert!(matches!(
            registry.call(&ctx, "no_such_tool", json!({})).await,
            Err(PluginError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn call_rejects_args_missing_a_required_field() {
        let registry = ToolRegistry::build(&[], planner()).unwrap();
        let (ctx, _mock, _dir) = crate::test_support::context().await;

        // core_remove_preference requires "preference_id".
        assert!(matches!(
            registry
                .call(&ctx, "core_remove_preference", json!({}))
                .await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    #[test]
    fn validate_args_accepts_a_matching_object() {
        let schema = json!({
            "type": "object",
            "required": ["kg"],
            "properties": { "kg": {"type": "number"}, "note": {"type": "string"} }
        });
        assert!(validate_args(&schema, &json!({"kg": 60.0})).is_ok());
        assert!(validate_args(&schema, &json!({"kg": 60.0, "note": "x"})).is_ok());
    }

    #[test]
    fn validate_args_rejects_missing_required_field() {
        let schema = json!({"type": "object", "required": ["kg"], "properties": {}});
        assert!(validate_args(&schema, &json!({})).is_err());
    }

    #[test]
    fn validate_args_rejects_wrong_type() {
        let schema = json!({"type": "object", "properties": {"kg": {"type": "number"}}});
        assert!(validate_args(&schema, &json!({"kg": "not a number"})).is_err());
    }

    #[test]
    fn validate_args_rejects_value_not_in_enum() {
        let schema = json!({
            "type": "object",
            "properties": { "meal_type": {"type": "string", "enum": ["breakfast", "lunch"]} }
        });
        assert!(validate_args(&schema, &json!({"meal_type": "dinner"})).is_err());
        assert!(validate_args(&schema, &json!({"meal_type": "lunch"})).is_ok());
    }

    #[test]
    fn validate_args_checks_array_items() {
        let schema = json!({
            "type": "object",
            "properties": { "items": {"type": "array", "items": {"type": "number"}} }
        });
        assert!(validate_args(&schema, &json!({"items": [1, 2, 3]})).is_ok());
        assert!(validate_args(&schema, &json!({"items": [1, "two", 3]})).is_err());
    }
}
