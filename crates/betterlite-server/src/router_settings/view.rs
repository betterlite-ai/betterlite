use std::collections::BTreeMap;

use betterlite_config::sections::fallback_plans::chains::FallbackChainEntry;
use betterlite_config::sections::settings::{JsonValue, Section, SettingsStore};
use betterlite_config::types::Config;

use self::configured::{configured_values, default_litellm_params};
use super::catalog::strategy_name;

mod configured;

type Attribute = fn(&Config) -> (&'static str, JsonValue);

// every router attribute the endpoint reports when the config does not write it
static DEFAULTS: [Attribute; 22] = [
    |config| {
        (
            "routing_strategy",
            JsonValue::from(strategy_name(config.sections.router_core.routing_strategy)),
        )
    },
    |_| {
        (
            "routing_strategy_args",
            JsonValue::Object(BTreeMap::new().into_iter().collect()),
        )
    },
    |_| ("routing_groups", JsonValue::Array(Vec::new())),
    |config| {
        (
            "num_retries",
            JsonValue::from(config.sections.retry_policy.num_retries),
        )
    },
    |config| {
        (
            "timeout",
            JsonValue::from(config.sections.routing_timeouts.global.seconds),
        )
    },
    |_| ("stream_timeout", JsonValue::Null),
    |config| {
        (
            "max_fallbacks",
            JsonValue::from(config.sections.fallback_plans.max_fallbacks),
        )
    },
    |config| {
        (
            "fallbacks",
            list_or_null(&config.sections.fallback_plans.fallbacks),
        )
    },
    |config| {
        (
            "context_window_fallbacks",
            list_or_null(&config.sections.fallback_plans.context_window_fallbacks),
        )
    },
    |config| {
        (
            "content_policy_fallbacks",
            list_or_null(&config.sections.fallback_plans.content_policy_fallbacks),
        )
    },
    |config| {
        (
            "allowed_fails",
            JsonValue::from(config.sections.cooldown_decisions.allowed_fails),
        )
    },
    |config| {
        (
            "cooldown_time",
            number(config.sections.cooldown_decisions.cooldown_time),
        )
    },
    |config| {
        (
            "retry_after",
            number(config.sections.retry_policy.retry_after),
        )
    },
    |_| ("retry_policy", JsonValue::Null),
    |_| {
        (
            "model_group_alias",
            JsonValue::Object(BTreeMap::new().into_iter().collect()),
        )
    },
    |config| {
        (
            "enable_pre_call_checks",
            JsonValue::Bool(config.sections.static_eligibility.enable_pre_call_checks),
        )
    },
    |config| ("default_litellm_params", default_litellm_params(config)),
    |_| ("set_verbose", JsonValue::Bool(false)),
    |_| ("default_max_parallel_requests", JsonValue::Null),
    |_| ("enable_tag_filtering", JsonValue::Bool(false)),
    |_| ("tag_filtering_match_any", JsonValue::Bool(true)),
    |_| ("disable_cooldowns", JsonValue::Null),
];

// every router attribute the endpoint reports, with the config override winning over the computed default
#[must_use]
pub fn current_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = default_values(config);
    values.extend(configured_values(config));
    values
}

#[must_use]
pub fn settings_store(config: &Config) -> SettingsStore {
    let mut store = SettingsStore::from(Section::RouterSettings);
    store.load_yaml(configured_values(config));
    store
}

fn default_values(config: &Config) -> BTreeMap<String, JsonValue> {
    DEFAULTS
        .into_iter()
        .map(|attribute| attribute(config))
        .map(|(key, value)| (String::from(key), value))
        .collect()
}

// LiteLLM's Router holds None for a fallback list it was not given, and the config wins over that
fn list_or_null(entries: &[FallbackChainEntry]) -> JsonValue {
    if entries.is_empty() {
        JsonValue::Null
    } else {
        serde_json::to_value(entries).unwrap_or(JsonValue::Null)
    }
}

// a whole value is the YAML integer Python reports, and a fraction stays a float
pub(super) fn number(value: f64) -> JsonValue {
    let integer = value.is_finite() && value.fract() == 0.0_f64;
    if integer && let Ok(whole) = format!("{value:.0}").parse::<i64>() {
        return JsonValue::from(whole);
    }
    JsonValue::from(value)
}
