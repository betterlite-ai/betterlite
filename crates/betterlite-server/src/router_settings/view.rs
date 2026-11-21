use std::collections::BTreeMap;

use betterlite_config::sections::fallback_plans::chains::FallbackChainEntry;
use betterlite_config::sections::settings::{JsonValue, Section, SettingsStore};
use betterlite_config::types::Config;
use serde::Serialize;

use self::configured::{configured_values, default_litellm_params};
use super::catalog::strategy_name;

mod configured;

// every router attribute the endpoint reports, with the config override winning over the computed default
#[must_use]
pub fn current_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = configured_values(config);
    for defaults in [
        core_defaults(config),
        retry_defaults(config),
        fallback_defaults(config),
        cooldown_defaults(config),
        alias_defaults(config),
        switch_defaults(),
    ] {
        for (key, value) in defaults {
            values.entry(key).or_insert(value);
        }
    }
    values
}

#[must_use]
pub fn settings_store(config: &Config) -> SettingsStore {
    let mut store = SettingsStore::from(Section::RouterSettings);
    store.load_yaml(configured_values(config));
    store
}

fn core_defaults(config: &Config) -> BTreeMap<String, JsonValue> {
    BTreeMap::from([
        (
            String::from("routing_strategy"),
            text(strategy_name(config.sections.router_core.routing_strategy)),
        ),
        (
            String::from("routing_strategy_args"),
            JsonValue::Object(Default::default()),
        ),
        (String::from("routing_groups"), JsonValue::Array(Vec::new())),
    ])
}

fn retry_defaults(config: &Config) -> BTreeMap<String, JsonValue> {
    BTreeMap::from([
        (
            String::from("num_retries"),
            JsonValue::from(config.sections.retry_policy.num_retries),
        ),
        (
            String::from("timeout"),
            JsonValue::from(config.sections.routing_timeouts.global.seconds),
        ),
        (String::from("stream_timeout"), JsonValue::Null),
    ])
}

fn fallback_defaults(config: &Config) -> BTreeMap<String, JsonValue> {
    let plans = &config.sections.fallback_plans;
    BTreeMap::from([
        (
            String::from("max_fallbacks"),
            JsonValue::from(plans.max_fallbacks),
        ),
        (String::from("fallbacks"), list_or_null(&plans.fallbacks)),
        (
            String::from("context_window_fallbacks"),
            list_or_null(&plans.context_window_fallbacks),
        ),
        (
            String::from("content_policy_fallbacks"),
            list_or_null(&plans.content_policy_fallbacks),
        ),
    ])
}

fn cooldown_defaults(config: &Config) -> BTreeMap<String, JsonValue> {
    BTreeMap::from([
        (
            String::from("allowed_fails"),
            JsonValue::from(config.sections.cooldown_decisions.allowed_fails),
        ),
        (
            String::from("cooldown_time"),
            number(config.sections.cooldown_decisions.cooldown_time),
        ),
        (
            String::from("retry_after"),
            number(config.sections.retry_policy.retry_after),
        ),
        (String::from("retry_policy"), JsonValue::Null),
    ])
}

fn alias_defaults(config: &Config) -> BTreeMap<String, JsonValue> {
    BTreeMap::from([
        (
            String::from("model_group_alias"),
            JsonValue::Object(Default::default()),
        ),
        (
            String::from("enable_pre_call_checks"),
            JsonValue::Bool(config.sections.static_eligibility.enable_pre_call_checks),
        ),
        (
            String::from("default_litellm_params"),
            default_litellm_params(config),
        ),
    ])
}

fn switch_defaults() -> BTreeMap<String, JsonValue> {
    BTreeMap::from([
        (String::from("set_verbose"), JsonValue::Bool(false)),
        (
            String::from("default_max_parallel_requests"),
            JsonValue::Null,
        ),
        (String::from("enable_tag_filtering"), JsonValue::Bool(false)),
        (
            String::from("tag_filtering_match_any"),
            JsonValue::Bool(true),
        ),
        (String::from("disable_cooldowns"), JsonValue::Null),
    ])
}

// LiteLLM's Router holds None for a fallback list it was not given, and the config wins over that
fn list_or_null(entries: &[FallbackChainEntry]) -> JsonValue {
    if entries.is_empty() {
        JsonValue::Null
    } else {
        serialized(entries)
    }
}

// these shapes hold strings, numbers and maps of them, so their serialization cannot fail
pub(super) fn serialized<T: Serialize>(value: &T) -> JsonValue {
    serde_json::to_value(value).unwrap_or(JsonValue::Null)
}

pub(super) fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

// a whole value is the YAML integer Python reports, and a fraction stays a float
pub(super) fn number(value: f64) -> JsonValue {
    let integer = value.is_finite() && value.fract() == 0.0;
    if integer {
        if let Ok(whole) = format!("{value:.0}").parse::<i64>() {
            return JsonValue::from(whole);
        }
    }
    JsonValue::from(value)
}
