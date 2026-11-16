pub mod error;
pub mod integer;
pub mod retry_policy_value;
pub mod target;
pub mod update;

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

use crate::router_runtime_settings::error::RouterSettingsError;
use crate::router_runtime_settings::target::RouterSettingsTarget;

mod tests;

pub const UPDATABLE_ROUTER_SETTINGS: [&str; 18] = [
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouterRuntimeSettings {
    pub values: BTreeMap<String, JsonValue>,
    pub routing_groups_input: Option<JsonValue>,
}

// What get_settings reads from state that other tasks own
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsView {
    pub routing_groups: Vec<JsonValue>,
    pub latency_routing_args: Option<String>,
}

impl RouterRuntimeSettings {
    #[must_use]
    pub fn get_settings(&self, view: &SettingsView) -> BTreeMap<String, JsonValue> {
        let mut settings = BTreeMap::new();
        let latency_strategy = self
            .values
            .get("routing_strategy")
            .is_some_and(|strategy| strategy.as_str() == Some(LATENCY_STRATEGY));
        for name in SETTINGS_SHOWN {
            if let Some(value) = self.values.get(name) {
                settings.insert(name.to_owned(), value.clone());
            }
            if name == "routing_strategy_args"
                && latency_strategy
                && let Some(args) = &view.latency_routing_args
            {
                settings.insert(name.to_owned(), JsonValue::String(args.clone()));
            }
        }
        settings.insert(
            "routing_groups".to_owned(),
            JsonValue::Array(view.routing_groups.clone()),
        );
        settings
    }

    pub fn update_settings(
        &mut self,
        target: &mut dyn RouterSettingsTarget,
        kwargs: &[(String, JsonValue)],
    ) -> Result<(), RouterSettingsError> {
        let mut pass = update::UpdatePass {
            previous_strategy: self.values.get("routing_strategy").cloned(),
            rebuild_routing_groups: false,
            routing_args_updated: false,
        };
        for (name, value) in kwargs {
            if UPDATABLE_ROUTER_SETTINGS.contains(&name.as_str()) {
                update::apply_pair(self, target, kwargs, (name.as_str(), value), &mut pass)?;
            } else {
                tracing::debug!("Setting {name} is not allowed");
            }
        }
        if pass.routing_args_updated {
            target.apply_updated_strategy_args()?;
        }
        if pass.rebuild_routing_groups {
            let input = match kwargs.iter().find(|(name, _)| name == "routing_groups") {
                Some((_, groups)) if !groups.is_null() => Some(groups.clone()),
                Some(_) => None,
                None => self.routing_groups_input.clone(),
            };
            target.init_routing_groups(input.as_ref())?;
            self.routing_groups_input = input;
        }
        let values = &self.values;
        tracing::debug!("Updated Router settings: {values:?}");
        Ok(())
    }
}
