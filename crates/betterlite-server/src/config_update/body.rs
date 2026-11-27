use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use super::SettingsMap;
use crate::search_tool_routes::body::{BodyError, decode_body};

type Weights = BTreeMap<String, BTreeMap<String, f64>>;
type Aliases = BTreeMap<String, JsonValue>;

// ConfigYAML: the four sections the route writes, in any order, with unknown top-level keys ignored
#[derive(Debug, Default, Deserialize)]
struct ConfigUpdateBody {
    #[serde(default)]
    general_settings: Option<SettingsMap>,
    #[serde(default)]
    environment_variables: Option<SettingsMap>,
    #[serde(default)]
    litellm_settings: Option<SettingsMap>,
    #[serde(default)]
    router_settings: Option<RouterSettingsBody>,
}

// the same body again as plain JSON, so router_settings is checked from the raw body and weights: null still reaches the allowlist
#[derive(Debug, Deserialize)]
struct RawRouterSettings {
    #[serde(default)]
    router_settings: Option<SettingsMap>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DecodedConfigUpdate {
    pub general_settings: Option<SettingsMap>,
    pub environment_variables: Option<SettingsMap>,
    pub litellm_settings: Option<SettingsMap>,
    pub router_settings: Option<RouterSettingsBody>,
    pub raw_router_settings: Option<SettingsMap>,
}

// UpdateRouterConfig: the declared fields validate the body the way pydantic does and dump as the typed half of the merge
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct RouterSettingsBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_strategy_args: Option<SettingsMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_strategy: Option<String>,
    // RoutingGroup validation belongs to parse_routing_groups, which answers with its own 400
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_groups: Option<Vec<JsonValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetryPolicyBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_group_retry_policy: Option<BTreeMap<String, RetryPolicyBody>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_group_affinity_config: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_fails: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_time: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_retries: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallbacks: Option<Vec<SettingsMap>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_fallbacks: Option<Vec<SettingsMap>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "aliases"
    )]
    pub model_group_alias: Option<Aliases>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_tag_filtering: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "weights"
    )]
    pub weights: Option<Weights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_routing_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optional_pre_call_checks: Option<Vec<PreCallCheck>>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct RetryPolicyBody {
    #[serde(
        default,
        rename = "BadRequestErrorRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub bad_request_error_retries: Option<i64>,
    #[serde(
        default,
        rename = "AuthenticationErrorRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub authentication_error_retries: Option<i64>,
    #[serde(
        default,
        rename = "TimeoutErrorRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_error_retries: Option<i64>,
    #[serde(
        default,
        rename = "RateLimitErrorRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub rate_limit_error_retries: Option<i64>,
    #[serde(
        default,
        rename = "ContentPolicyViolationErrorRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub content_policy_retries: Option<i64>,
    #[serde(
        default,
        rename = "InternalServerErrorRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub internal_server_error_retries: Option<i64>,
    #[serde(
        default,
        rename = "ServiceUnavailableErrorRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub service_unavailable_retries: Option<i64>,
    #[serde(
        default,
        rename = "NotFoundErrorRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub not_found_error_retries: Option<i64>,
    #[serde(
        default,
        rename = "DefaultRetries",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_retries: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreCallCheck {
    PromptCaching,
    RouterBudgetLimiting,
    ResponsesApiDeploymentCheck,
    DeploymentAffinity,
    SessionAffinity,
    #[serde(rename = "forward_client_headers_by_model_group")]
    ForwardClientHeaders,
    EnforceModelRateLimits,
    EncryptedContentAffinity,
}

// FastAPI validates the body before the handler runs, so a bad body is 422 no matter who asked
pub fn decode_config_update(bytes: &[u8]) -> Result<DecodedConfigUpdate, BodyError> {
    let body: ConfigUpdateBody = decode_body(bytes)?;
    let raw: RawRouterSettings = decode_body(bytes)?;
    Ok(DecodedConfigUpdate {
        general_settings: body.general_settings,
        environment_variables: body.environment_variables,
        litellm_settings: body.litellm_settings,
        router_settings: body.router_settings,
        raw_router_settings: raw.router_settings,
    })
}

// RouterWeights: every identifier holds a non-whitespace character, every weight is finite and non-negative, and each nonempty group can route
pub(crate) fn weights<'de, D>(deserializer: D) -> Result<Option<Weights>, D::Error>
where
    D: Deserializer<'de>,
{
    let weights = Option::<Weights>::deserialize(deserializer)?;
    if let Some(problem) = weights.as_ref().and_then(weights_problem) {
        return Err(D::Error::custom(problem));
    }
    Ok(weights)
}

fn weights_problem(weights: &Weights) -> Option<&'static str> {
    for (group, entries) in weights {
        if group.trim().is_empty() {
            return Some("weights group names must contain a non-whitespace character");
        }
        if entries.keys().any(|model| model.trim().is_empty()) {
            return Some("weights model names must contain a non-whitespace character");
        }
        if entries
            .values()
            .any(|weight| !weight.is_finite() || *weight < 0.0_f64)
        {
            return Some("weights must be non-negative finite numbers");
        }
        if !entries.is_empty() && !entries.values().any(|weight| *weight > 0.0_f64) {
            return Some("Each nonempty weights group must contain at least one positive weight");
        }
    }
    None
}

// model_group_alias: Dict[str, Union[str, dict]]
pub(crate) fn aliases<'de, D>(deserializer: D) -> Result<Option<Aliases>, D::Error>
where
    D: Deserializer<'de>,
{
    let aliases = Option::<Aliases>::deserialize(deserializer)?;
    if let Some(problem) = aliases.as_ref().and_then(aliases_problem) {
        return Err(D::Error::custom(problem));
    }
    Ok(aliases)
}

fn aliases_problem(aliases: &Aliases) -> Option<&'static str> {
    for value in aliases.values() {
        match value {
            JsonValue::String(_) | JsonValue::Object(_) => {}
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
                return Some("model_group_alias values must be a string or an object");
            }
        }
    }
    None
}
