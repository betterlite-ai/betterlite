use std::collections::BTreeMap;

use betterlite_config::sections::deployment_catalog::GroupAlias;
use betterlite_config::sections::retry_policy::RetryPolicySettings;
use betterlite_config::sections::routing_groups::RoutingGroupSettings;
use betterlite_config::sections::routing_timeouts::RouterTimeouts;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::Config;

use self::strategy_arguments::strategy_args;
use super::{number, serialized, text};
use crate::router_settings::catalog::strategy_name;

mod strategy_arguments;

// the router_settings keys the loaded config proves were written, in the shape the config merge reports
pub(super) fn configured_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    for written in [
        router_values(config),
        fallback_values(config),
        timeout_values(config),
        retry_values(config),
        alias_values(config),
        switch_values(config),
        routing_group_values(config),
        strategy_values(config),
        parallel_values(config),
    ] {
        values.extend(written);
    }
    values
}

fn router_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    let router = &config.router_settings;
    if let Some(strategy) = router.routing_strategy {
        values.insert(
            String::from("routing_strategy"),
            text(strategy_name(strategy)),
        );
    }
    if let Some(retries) = router.num_retries {
        values.insert(String::from("num_retries"), JsonValue::from(retries));
    }
    if let Some(timeout) = router.timeout {
        values.insert(String::from("timeout"), number(timeout));
    }
    if let Some(fails) = router.allowed_fails {
        values.insert(String::from("allowed_fails"), JsonValue::from(fails));
    }
    if let Some(cooldown) = router.cooldown_time {
        values.insert(String::from("cooldown_time"), number(cooldown));
    }
    values
}

fn fallback_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    let plans = &config.sections.fallback_plans;
    if !config.router_settings.fallbacks.is_empty() {
        let fallbacks = &config.router_settings.fallbacks;
        values.insert(String::from("fallbacks"), serialized(fallbacks));
    }
    if !plans.context_window_fallbacks.is_empty() {
        let entries = &plans.context_window_fallbacks;
        values.insert(
            String::from("context_window_fallbacks"),
            serialized(entries),
        );
    }
    if !plans.content_policy_fallbacks.is_empty() {
        let entries = &plans.content_policy_fallbacks;
        values.insert(
            String::from("content_policy_fallbacks"),
            serialized(entries),
        );
    }
    values
}

fn timeout_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    let timeouts = &config.sections.routing_timeouts.router;
    if let Some(stream_timeout) = timeouts.stream_timeout {
        values.insert(String::from("stream_timeout"), number(stream_timeout));
    }
    if timeouts.default_timeout.is_some() || timeouts.default_stream_timeout.is_some() {
        values.insert(
            String::from("default_litellm_params"),
            configured_default_params(timeouts),
        );
    }
    values
}

fn retry_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    if let Some(policy) = &config.sections.retry_policy.retry_policy {
        values.insert(String::from("retry_policy"), policy_value(policy));
    }
    values
}

fn alias_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    let aliases = &config.sections.deployment_catalog.model_group_alias;
    if aliases.is_empty() {
        return values;
    }
    let written = aliases
        .iter()
        .map(|(name, alias)| (name.clone(), alias_value(alias)))
        .collect();
    values.insert(
        String::from("model_group_alias"),
        JsonValue::Object(written),
    );
    values
}

// a string alias stays a string, and an item that hides the alias keeps its two declared keys
fn alias_value(alias: &GroupAlias) -> JsonValue {
    if !alias.hidden {
        return text(&alias.model);
    }
    JsonValue::Object(
        [
            (String::from("model"), text(&alias.model)),
            (String::from("hidden"), JsonValue::Bool(true)),
        ]
        .into_iter()
        .collect(),
    )
}

fn switch_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    if config.sections.static_eligibility.enable_pre_call_checks {
        values.insert(
            String::from("enable_pre_call_checks"),
            JsonValue::Bool(true),
        );
    }
    if config.sections.cooldown_decisions.disable_cooldowns {
        values.insert(String::from("disable_cooldowns"), JsonValue::Bool(true));
    }
    values
}

fn routing_group_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    if let Some(groups) = &config.sections.routing_groups {
        values.insert(String::from("routing_groups"), group_values(&groups.groups));
    }
    values
}

fn strategy_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    let args = strategy_args(config);
    if !args.is_empty() {
        values.insert(
            String::from("routing_strategy_args"),
            JsonValue::Object(args.into_iter().collect()),
        );
    }
    values
}

fn parallel_values(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut values = BTreeMap::new();
    if let Some(parallel) = config
        .sections
        .deployment_clients
        .default_max_parallel_requests
    {
        values.insert(
            String::from("default_max_parallel_requests"),
            JsonValue::from(parallel),
        );
    }
    values
}

fn group_values(groups: &[RoutingGroupSettings]) -> JsonValue {
    JsonValue::Array(groups.iter().map(group_value).collect())
}

fn group_value(group: &RoutingGroupSettings) -> JsonValue {
    let mut value = BTreeMap::new();
    value.insert(String::from("group_name"), text(&group.group_name));
    value.insert(
        String::from("models"),
        JsonValue::Array(group.models.iter().map(|model| text(model)).collect()),
    );
    value.insert(
        String::from("routing_strategy"),
        text(&group.routing_strategy),
    );
    if let Some(args) = &group.routing_strategy_args {
        value.insert(String::from("routing_strategy_args"), serialized(args));
    }
    if let Some(priorities) = &group.model_priorities {
        value.insert(
            String::from("model_priorities"),
            JsonValue::Object(
                priorities
                    .iter()
                    .map(|(model, priority)| (model.clone(), JsonValue::from(*priority)))
                    .collect(),
            ),
        );
    }
    JsonValue::Object(value.into_iter().collect())
}

// the object Router builds for a run without router_settings.default_litellm_params
pub(super) fn default_litellm_params(config: &Config) -> JsonValue {
    let mut params = BTreeMap::new();
    params.insert(String::from("max_retries"), JsonValue::from(0_i64));
    params.insert(
        String::from("metadata"),
        JsonValue::Object(
            [(String::from("caching_groups"), JsonValue::Null)]
                .into_iter()
                .collect(),
        ),
    );
    let timeout = config
        .router_settings
        .timeout
        .map_or(JsonValue::Null, number);
    params.insert(String::from("timeout"), timeout);
    JsonValue::Object(params.into_iter().collect())
}

fn configured_default_params(timeouts: &RouterTimeouts) -> JsonValue {
    let mut params = BTreeMap::new();
    if let Some(timeout) = timeouts.default_timeout {
        params.insert(String::from("timeout"), number(timeout));
    }
    if let Some(stream_timeout) = timeouts.default_stream_timeout {
        params.insert(String::from("stream_timeout"), number(stream_timeout));
    }
    JsonValue::Object(params.into_iter().collect())
}

fn policy_value(policy: &RetryPolicySettings) -> JsonValue {
    let mut value = BTreeMap::new();
    let fields = [
        ("BadRequestErrorRetries", policy.bad_request),
        ("AuthenticationErrorRetries", policy.authentication),
        ("TimeoutErrorRetries", policy.timeout),
        ("RateLimitErrorRetries", policy.rate_limit),
        ("ContentPolicyViolationErrorRetries", policy.content_policy),
        ("InternalServerErrorRetries", policy.internal_server),
        ("ServiceUnavailableErrorRetries", policy.service_unavailable),
        ("NotFoundErrorRetries", policy.not_found),
        ("DefaultRetries", policy.fallback),
    ];
    for (name, retries) in fields {
        if let Some(retries) = retries {
            value.insert(String::from(name), JsonValue::from(retries));
        }
    }
    JsonValue::Object(value.into_iter().collect())
}
