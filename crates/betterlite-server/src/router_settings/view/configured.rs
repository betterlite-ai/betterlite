use std::collections::BTreeMap;
use std::iter::once;

use betterlite_config::sections::deployment_catalog::GroupAlias;
use betterlite_config::sections::retry_policy::RetryPolicySettings;
use betterlite_config::sections::routing_groups::RoutingGroupSettings;
use betterlite_config::sections::routing_timeouts::RouterTimeouts;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::Config;

use self::strategy_arguments::strategy_args;
use super::number;
use crate::router_settings::catalog::strategy_name;

mod strategy_arguments;

type Writer = fn(&Config) -> Option<(&'static str, JsonValue)>;

// the router_settings keys the loaded config proves were written, in the shape the config merge reports
static WRITERS: [Writer; 17] = [
    |config| {
        config
            .router_settings
            .routing_strategy
            .map(|strategy| ("routing_strategy", JsonValue::from(strategy_name(strategy))))
    },
    |config| {
        config
            .router_settings
            .num_retries
            .map(|retries| ("num_retries", JsonValue::from(retries)))
    },
    |config| {
        config
            .router_settings
            .timeout
            .map(|timeout| ("timeout", number(timeout)))
    },
    |config| {
        config
            .router_settings
            .allowed_fails
            .map(|fails| ("allowed_fails", JsonValue::from(fails)))
    },
    |config| {
        config
            .router_settings
            .cooldown_time
            .map(|cooldown| ("cooldown_time", number(cooldown)))
    },
    |config| {
        let fallbacks = &config.router_settings.fallbacks;
        (!fallbacks.is_empty()).then(|| {
            (
                "fallbacks",
                serde_json::to_value(fallbacks).unwrap_or(JsonValue::Null),
            )
        })
    },
    |config| {
        let entries = &config.sections.fallback_plans.context_window_fallbacks;
        (!entries.is_empty()).then(|| {
            (
                "context_window_fallbacks",
                serde_json::to_value(entries).unwrap_or(JsonValue::Null),
            )
        })
    },
    |config| {
        let entries = &config.sections.fallback_plans.content_policy_fallbacks;
        (!entries.is_empty()).then(|| {
            (
                "content_policy_fallbacks",
                serde_json::to_value(entries).unwrap_or(JsonValue::Null),
            )
        })
    },
    |config| {
        config
            .sections
            .routing_timeouts
            .router
            .stream_timeout
            .map(|timeout| ("stream_timeout", number(timeout)))
    },
    |config| {
        let timeouts = &config.sections.routing_timeouts.router;
        let configured =
            timeouts.default_timeout.is_some() || timeouts.default_stream_timeout.is_some();
        configured.then(|| {
            (
                "default_litellm_params",
                configured_default_params(timeouts),
            )
        })
    },
    |config| {
        config
            .sections
            .retry_policy
            .retry_policy
            .as_ref()
            .map(|policy| ("retry_policy", policy_value(policy)))
    },
    |config| {
        let aliases = &config.sections.deployment_catalog.model_group_alias;
        (!aliases.is_empty()).then(|| {
            (
                "model_group_alias",
                JsonValue::Object(
                    aliases
                        .iter()
                        .map(|(name, alias)| (name.clone(), alias_value(alias)))
                        .collect(),
                ),
            )
        })
    },
    |config| {
        config
            .sections
            .static_eligibility
            .enable_pre_call_checks
            .then_some(("enable_pre_call_checks", JsonValue::Bool(true)))
    },
    |config| {
        config
            .sections
            .cooldown_decisions
            .disable_cooldowns
            .then_some(("disable_cooldowns", JsonValue::Bool(true)))
    },
    |config| {
        config.sections.routing_groups.as_ref().map(|groups| {
            (
                "routing_groups",
                JsonValue::Array(groups.groups.iter().map(group_value).collect()),
            )
        })
    },
    |config| {
        let args = strategy_args(config);
        (!args.is_empty()).then(|| {
            (
                "routing_strategy_args",
                JsonValue::Object(args.into_iter().collect()),
            )
        })
    },
    |config| {
        config
            .sections
            .deployment_clients
            .default_max_parallel_requests
            .map(|parallel| ("default_max_parallel_requests", JsonValue::from(parallel)))
    },
];

pub(super) fn configured_values(config: &Config) -> BTreeMap<String, JsonValue> {
    WRITERS
        .into_iter()
        .filter_map(|writer| writer(config))
        .map(|(key, value)| (String::from(key), value))
        .collect()
}

// a string alias stays a string, and an item that hides the alias keeps its two declared keys
fn alias_value(alias: &GroupAlias) -> JsonValue {
    if !alias.hidden {
        return JsonValue::from(alias.model.as_str());
    }
    JsonValue::Object(
        [
            (String::from("model"), JsonValue::from(alias.model.as_str())),
            (String::from("hidden"), JsonValue::Bool(true)),
        ]
        .into_iter()
        .collect(),
    )
}

fn group_value(group: &RoutingGroupSettings) -> JsonValue {
    let mut value = BTreeMap::new();
    value.insert(
        String::from("group_name"),
        JsonValue::from(group.group_name.as_str()),
    );
    value.insert(
        String::from("models"),
        JsonValue::Array(
            group
                .models
                .iter()
                .map(|model| JsonValue::from(model.as_str()))
                .collect(),
        ),
    );
    value.insert(
        String::from("routing_strategy"),
        JsonValue::from(group.routing_strategy.as_str()),
    );
    if let Some(args) = &group.routing_strategy_args {
        value.insert(
            String::from("routing_strategy_args"),
            serde_json::to_value(args).unwrap_or(JsonValue::Null),
        );
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
        JsonValue::Object(once((String::from("caching_groups"), JsonValue::Null)).collect()),
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
        if let Some(count) = retries {
            value.insert(String::from(name), JsonValue::from(count));
        }
    }
    JsonValue::Object(value.into_iter().collect())
}
