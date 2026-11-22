use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::RoutingStrategy;

// the catalog mirrors ROUTER_SETTINGS_FIELDS and ROUTING_STRATEGY_DESCRIPTIONS in litellm/types/management_endpoints/router_settings_endpoints.py
#[derive(Clone, Copy)]
pub(super) enum DefaultValue {
    Null,
    Text(&'static str),
    Integer(i64),
    Boolean(bool),
    List,
    Map,
}

impl DefaultValue {
    pub(super) fn json(self) -> JsonValue {
        match self {
            Self::Null => JsonValue::Null,
            Self::Text(text) => JsonValue::String(text.to_owned()),
            Self::Integer(value) => JsonValue::from(value),
            Self::Boolean(flag) => JsonValue::Bool(flag),
            Self::List => JsonValue::Array(Vec::new()),
            Self::Map => JsonValue::Object(BTreeMap::new().into_iter().collect()),
        }
    }
}

pub(super) struct FieldDefinition {
    pub(super) name: &'static str,
    pub(super) kind: &'static str,
    pub(super) description: &'static str,
    pub(super) default: DefaultValue,
    pub(super) ui_name: &'static str,
    pub(super) link: Option<&'static str>,
}

pub(super) const FIELDS: [FieldDefinition; 22] = [
    FieldDefinition {
        name: "routing_strategy",
        kind: "String",
        description: "Routing strategy to use for load balancing across deployments",
        default: DefaultValue::Text("simple-shuffle"),
        ui_name: "Routing Strategy",
        link: None,
    },
    FieldDefinition {
        name: "routing_strategy_args",
        kind: "Dictionary",
        description: "Arguments to pass to the routing strategy (e.g., ttl, lowest_latency_buffer for latency-based-routing)",
        default: DefaultValue::Map,
        ui_name: "Routing Strategy Args",
        link: None,
    },
    FieldDefinition {
        name: "routing_groups",
        kind: "List",
        description: "Named subsets of model_names that share a routing strategy. Models not claimed by an explicit group fall through to the top-level routing_strategy.",
        default: DefaultValue::List,
        ui_name: "Routing Groups",
        link: None,
    },
    FieldDefinition {
        name: "num_retries",
        kind: "Integer",
        description: "Number of retries for failed requests",
        default: DefaultValue::Integer(0),
        ui_name: "Number of Retries",
        link: None,
    },
    FieldDefinition {
        name: "timeout",
        kind: "Float",
        description: "Timeout for requests in seconds",
        default: DefaultValue::Null,
        ui_name: "Timeout",
        link: None,
    },
    FieldDefinition {
        name: "stream_timeout",
        kind: "Float",
        description: "Timeout for streaming requests in seconds",
        default: DefaultValue::Null,
        ui_name: "Stream Timeout",
        link: None,
    },
    FieldDefinition {
        name: "max_fallbacks",
        kind: "Integer",
        description: "Maximum number of fallbacks to try before exiting the call",
        default: DefaultValue::Integer(5),
        ui_name: "Max Fallbacks",
        link: None,
    },
    FieldDefinition {
        name: "fallbacks",
        kind: "List",
        description: "List of fallback model mappings",
        default: DefaultValue::List,
        ui_name: "Fallbacks",
        link: None,
    },
    FieldDefinition {
        name: "context_window_fallbacks",
        kind: "List",
        description: "List of fallback models for context window errors",
        default: DefaultValue::List,
        ui_name: "Context Window Fallbacks",
        link: None,
    },
    FieldDefinition {
        name: "content_policy_fallbacks",
        kind: "List",
        description: "List of fallback models for content policy errors",
        default: DefaultValue::List,
        ui_name: "Content Policy Fallbacks",
        link: None,
    },
    FieldDefinition {
        name: "allowed_fails",
        kind: "Integer",
        description: "Number of times a deployment can fail before being added to cooldown",
        default: DefaultValue::Null,
        ui_name: "Allowed Fails",
        link: None,
    },
    FieldDefinition {
        name: "cooldown_time",
        kind: "Float",
        description: "Time in seconds to cooldown a deployment after failure",
        default: DefaultValue::Null,
        ui_name: "Cooldown Time",
        link: None,
    },
    FieldDefinition {
        name: "retry_after",
        kind: "Integer",
        description: "Minimum time to wait before retrying a failed request in seconds",
        default: DefaultValue::Integer(0),
        ui_name: "Retry After",
        link: None,
    },
    FieldDefinition {
        name: "retry_policy",
        kind: "Dictionary",
        description: "Custom retry policy for different exception types",
        default: DefaultValue::Null,
        ui_name: "Retry Policy",
        link: None,
    },
    FieldDefinition {
        name: "model_group_alias",
        kind: "Dictionary",
        description: "Aliases for model groups",
        default: DefaultValue::Map,
        ui_name: "Model Group Alias",
        link: None,
    },
    FieldDefinition {
        name: "enable_pre_call_checks",
        kind: "Boolean",
        description: "Enable pre-call checks before routing requests",
        default: DefaultValue::Boolean(false),
        ui_name: "Enable Pre-call Checks",
        link: None,
    },
    FieldDefinition {
        name: "default_litellm_params",
        kind: "Dictionary",
        description: "Default parameters for Router.chat.completion.create",
        default: DefaultValue::Null,
        ui_name: "Default LiteLLM Params",
        link: None,
    },
    FieldDefinition {
        name: "set_verbose",
        kind: "Boolean",
        description: "Enable verbose logging for router",
        default: DefaultValue::Boolean(false),
        ui_name: "Verbose Logging",
        link: None,
    },
    FieldDefinition {
        name: "default_max_parallel_requests",
        kind: "Integer",
        description: "Default maximum parallel requests across all deployments",
        default: DefaultValue::Null,
        ui_name: "Max Parallel Requests",
        link: None,
    },
    FieldDefinition {
        name: "enable_tag_filtering",
        kind: "Boolean",
        description: "Enable tag-based routing to route requests based on tags",
        default: DefaultValue::Boolean(false),
        ui_name: "Enable Tag Filtering",
        link: Some("https://docs.litellm.ai/docs/proxy/tag_routing"),
    },
    FieldDefinition {
        name: "tag_filtering_match_any",
        kind: "Boolean",
        description: "Match any tag instead of all tags for tag-based routing",
        default: DefaultValue::Boolean(true),
        ui_name: "Tag Filtering Match Any",
        link: None,
    },
    FieldDefinition {
        name: "disable_cooldowns",
        kind: "Boolean",
        description: "Disable cooldown mechanism for failed deployments",
        default: DefaultValue::Null,
        ui_name: "Disable Cooldowns",
        link: None,
    },
];

// RoutingStrategyName's Literal order in litellm/types/litellm_params.py, which the endpoint reads off Router.__init__
pub(super) const STRATEGIES: [RoutingStrategy; 7] = [
    RoutingStrategy::SimpleShuffle,
    RoutingStrategy::LeastBusy,
    RoutingStrategy::UsageBasedRouting,
    RoutingStrategy::LatencyBasedRouting,
    RoutingStrategy::CostBasedRouting,
    RoutingStrategy::UsageBasedRoutingV2,
    RoutingStrategy::Lar1,
];

pub(super) const PRIORITY_GROUP_STRATEGY: &str = "priority";

pub(super) const STRATEGY_DESCRIPTIONS: [(&str, &str); 7] = [
    (
        "priority",
        "Routes group calls to the lowest-priority-number available model, with failover to higher numbers. Equal priorities share traffic. Direct member calls keep their existing policy.",
    ),
    (
        "simple-shuffle",
        "Randomly picks a deployment from the list. Simple and fast.",
    ),
    (
        "least-busy",
        "Routes to the deployment with the lowest number of ongoing requests.",
    ),
    (
        "latency-based-routing",
        "Routes to the deployment with the lowest latency over a sliding window.",
    ),
    (
        "cost-based-routing",
        "Routes to the deployment with the lowest cost per token.",
    ),
    (
        "usage-based-routing",
        "Routes to the deployment with the lowest TPM (Tokens Per Minute) usage. (deprecated)",
    ),
    (
        "usage-based-routing-v2",
        "Improved version of usage-based routing with better tracking.",
    ),
];

pub(super) fn field_default(name: &str) -> JsonValue {
    FIELDS
        .iter()
        .find(|field| field.name == name)
        .map_or(JsonValue::Null, |field| field.default.json())
}
