use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::router_runtime_settings::UPDATABLE_ROUTER_SETTINGS;
use betterlite_router::routing_groups::group::parse_routing_groups;

use super::body::RouterSettingsBody;
use super::{ConfigUpdateError, SettingsMap};

const ROUTING_GROUPS: &str = "routing_groups";

// Router.get_valid_args(), the keyword names Router.__init__ declares
pub const ROUTER_INIT_ARGUMENTS: [&str; 58] = [
    "model_list",
    "assistants_config",
    "search_tools",
    "guardrail_list",
    "redis_url",
    "redis_host",
    "redis_port",
    "redis_password",
    "redis_db",
    "cache_responses",
    "cache_kwargs",
    "caching_groups",
    "client_ttl",
    "polling_interval",
    "default_priority",
    "num_retries",
    "max_fallbacks",
    "timeout",
    "stream_timeout",
    "default_litellm_params",
    "default_max_parallel_requests",
    "set_verbose",
    "debug_level",
    "default_fallbacks",
    "fallbacks",
    "context_window_fallbacks",
    "content_policy_fallbacks",
    "model_group_alias",
    "enable_pre_call_checks",
    "enable_tag_filtering",
    "tag_filtering_match_any",
    "tag_routing_prefix",
    "plugins",
    "retry_after",
    "retry_policy",
    "model_group_retry_policy",
    "allowed_fails",
    "allowed_fails_policy",
    "cooldown_time",
    "disable_cooldowns",
    "routing_strategy",
    "optional_pre_call_checks",
    "routing_strategy_args",
    "routing_groups",
    "provider_budget_config",
    "alerting_config",
    "router_general_settings",
    "deployment_affinity_ttl_seconds",
    "model_group_affinity_config",
    "ignore_invalid_deployments",
    "enable_health_check_routing",
    "health_check_staleness_threshold",
    "health_check_ignore_transient_errors",
    "background_health_check_model_groups",
    "enable_weighted_failover",
    "fallback_access_check",
    "fallback_budget_check",
    "auto_router_capability_limit",
];

// ROUTER_SETTINGS_MANAGED_OUTSIDE_CONFIG: reachable, but never through a config update
pub const MANAGED_OUTSIDE_CONFIG: [&str; 8] = [
    "model_list",
    "search_tools",
    "assistants_config",
    "router_general_settings",
    "ignore_invalid_deployments",
    "fallback_access_check",
    "fallback_budget_check",
    "auto_router_capability_limit",
];

// the raw body's keys outside RUNTIME_UPDATABLE_ROUTER_SETTINGS | (get_valid_args() - managed), in sorted order
#[must_use]
pub fn unsupported(raw: &SettingsMap) -> Vec<String> {
    raw.keys()
        .filter(|name| !is_supported(name.as_str()))
        .cloned()
        .collect()
}

fn is_supported(name: &str) -> bool {
    UPDATABLE_ROUTER_SETTINGS.contains(&name)
        || (ROUTER_INIT_ARGUMENTS.contains(&name) && !MANAGED_OUTSIDE_CONFIG.contains(&name))
}

pub fn check(raw: &SettingsMap) -> Result<(), ConfigUpdateError> {
    let unsupported = unsupported(raw);
    if !unsupported.is_empty() {
        return Err(ConfigUpdateError::UnsupportedRouterSettings {
            names: unsupported.join(", "),
        });
    }
    check_groups(raw.get(ROUTING_GROUPS))
}

fn check_groups(value: Option<&JsonValue>) -> Result<(), ConfigUpdateError> {
    let groups = match value {
        None | Some(JsonValue::Null) => return Ok(()),
        Some(JsonValue::Array(groups)) => groups,
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Object(_),
        ) => {
            return Err(ConfigUpdateError::RoutingGroupsList);
        }
    };
    let mut parsed: Vec<JsonMap> = Vec::with_capacity(groups.len());
    for entry in groups {
        match entry {
            JsonValue::Object(fields) => parsed.push(
                fields
                    .iter()
                    .map(|(key, field)| (key.clone(), decision(field)))
                    .collect(),
            ),
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_) => {
                return Err(ConfigUpdateError::RoutingGroupsEntry);
            }
        }
    }
    let _outcome = parse_routing_groups(Some(parsed.as_slice()), &BTreeSet::new())?;
    Ok(())
}

// the typed half of the merge: the sent declared fields, with nulls dropped the way exclude_none drops them
pub fn typed_settings(settings: &RouterSettingsBody) -> Result<SettingsMap, ConfigUpdateError> {
    match serde_json::to_value(settings)? {
        JsonValue::Object(fields) => Ok(fields.into_iter().collect()),
        // a struct always serializes to an object; the other shapes cannot come out of serde
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Ok(SettingsMap::new()),
    }
}

// typed_router_settings, then every raw key the typed dump lacks whose value is not null
#[must_use]
pub fn updates(typed: SettingsMap, extra: Option<&SettingsMap>) -> SettingsMap {
    let mut updates = typed;
    let Some(raw) = extra else {
        return updates;
    };
    for (key, value) in raw {
        if !updates.contains_key(key) && !value.is_null() {
            updates.insert(key.clone(), value.clone());
        }
    }
    updates
}

fn decision(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(decision).collect()),
        JsonValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), decision(field)))
                .collect(),
        ),
    }
}
