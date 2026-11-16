pub mod error;
pub mod integer;
pub mod retry_policy_value;
pub mod target;
pub mod update;

use serde_json::{Map, Value};

mod tests;

pub const RUNTIME_UPDATABLE_ROUTER_SETTINGS: [&str; 18] = [
    "routing_strategy_args",
    "routing_strategy",
    "routing_groups",
    "allowed_fails",
    "cooldown_time",
    "num_retries",
    "timeout",
    "max_retries",
    "retry_after",
    "fallbacks",
    "context_window_fallbacks",
    "retry_policy",
    "model_group_retry_policy",
    "model_group_alias",
    "enable_weighted_failover",
    "enable_tag_filtering",
    "tag_routing_prefix",
    "optional_pre_call_checks",
];

const SETTINGS_SHOWN: [&str; 16] = [
    "routing_strategy_args",
    "routing_strategy",
    "allowed_fails",
    "cooldown_time",
    "num_retries",
    "timeout",
    "max_retries",
    "retry_after",
    "fallbacks",
    "context_window_fallbacks",
    "model_group_retry_policy",
    "retry_policy",
    "model_group_alias",
    "enable_weighted_failover",
    "enable_tag_filtering",
    "tag_routing_prefix",
];

const LATENCY_STRATEGY: &str = "latency-based-routing";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouterRuntimeSettings {
    pub values: Map<String, Value>,
    pub routing_groups_input: Option<Value>,
}

// What get_settings reads from state that other tasks own
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsView {
    pub routing_groups: Vec<Value>,
    pub latency_routing_args: Option<String>,
}

impl RouterRuntimeSettings {
    #[must_use]
    pub fn get_settings(&self, view: &SettingsView) -> Map<String, Value> {
        let mut settings = Map::new();
        let latency_strategy = self.values.get("routing_strategy")
            == Some(&Value::String(LATENCY_STRATEGY.to_owned()));
        for name in SETTINGS_SHOWN {
            if let Some(value) = self.values.get(name) {
                settings.insert(name.to_owned(), value.clone());
            }
            if name == "routing_strategy_args"
                && latency_strategy
                && let Some(args) = &view.latency_routing_args
            {
                settings.insert(name.to_owned(), Value::String(args.clone()));
            }
        }
        settings.insert(
            "routing_groups".to_owned(),
            Value::Array(view.routing_groups.clone()),
        );
        settings
    }
}
