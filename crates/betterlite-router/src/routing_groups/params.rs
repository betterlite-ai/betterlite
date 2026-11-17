use std::collections::BTreeSet;

use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::filters::provider_rejectable_params;
use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_predicates::JsonMap;
use crate::router_predicates::params::{
    ParamSupport, declared_param_allowlist, deployment_accepts_param,
};

const REASONING_EFFORT: &str = "reasoning_effort";
const INTERNAL_CALL_ORIGIN_METADATA_KEY: &str = "internal_call_origin";
const AUTOROUTER_CLASSIFIER_CALL_ORIGIN: &str = "autorouter_classifier";
const NESTED_EFFORT_CARRIERS: [&str; 2] = ["output_config", "reasoning"];

// Router.TIER_PARAMS_NEVER_DROPPED's own names, on top of every litellm-owned kwarg.
const TIER_PARAMS_ALWAYS_KEPT: [&str; 7] = [
    "additional_drop_params",
    "drop_params",
    "messages",
    "model",
    "extra_headers",
    "max_tokens",
    "max_completion_tokens",
];

// The tier params that survive, and the OpenAI params no deployment behind the model declared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TierParams {
    pub params: JsonMap,
    pub dropped: BTreeSet<String>,
}

// _deployment_params_with_request_reasoning_override: an equivalent native carrier cannot outrank
// the request's own reasoning_effort, so every carrier a deployment configures is removed.
#[must_use]
pub fn deployment_params_with_request_reasoning_override(
    deployment_params: &JsonMap,
    request_kwargs: &JsonMap,
) -> JsonMap {
    let mut sanitized = deployment_params.clone();
    if absent_or_null(request_kwargs, REASONING_EFFORT) {
        return sanitized;
    }
    sanitized.pop("thinking");
    for carrier in NESTED_EFFORT_CARRIERS {
        pop_effort_from_nested_carrier(&mut sanitized, carrier);
    }
    let Some(extra_body) = sanitized.get("extra_body") else {
        return sanitized;
    };
    let Json::Object(extra_body) = extra_body else {
        return sanitized;
    };
    let mut extra_body = extra_body.clone();
    extra_body.pop(REASONING_EFFORT);
    extra_body.pop("thinking");
    for carrier in NESTED_EFFORT_CARRIERS {
        pop_effort_from_nested_carrier(&mut extra_body, carrier);
    }
    if extra_body.is_empty() {
        sanitized.pop("extra_body");
    } else {
        sanitized.insert("extra_body".to_owned(), Json::Object(extra_body));
    }
    sanitized
}

// _pop_effort_from_nested_carrier: drop the nested effort key, and the carrier once it is empty.
fn pop_effort_from_nested_carrier(params: &mut JsonMap, carrier: &str) {
    let Some(Json::Object(nested)) = params.get(carrier) else {
        return;
    };
    let mut sanitized = nested.clone();
    sanitized.pop("effort");
    if sanitized.is_empty() {
        params.pop(carrier);
    } else {
        params.insert(carrier.to_owned(), Json::Object(sanitized));
    }
}

// _is_classifier_internal_call: the auto-router classifier marked the request it made.
#[must_use]
pub fn is_classifier_internal_call(kwargs: &JsonMap) -> bool {
    ["metadata", "litellm_metadata"]
        .into_iter()
        .any(|key| match kwargs.get(key) {
            Some(Json::Object(fields)) => matches!(
                fields.get(INTERNAL_CALL_ORIGIN_METADATA_KEY),
                Some(Json::Text(origin)) if origin == AUTOROUTER_CLASSIFIER_CALL_ORIGIN
            ),
            None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                false
            }
        })
}

// _drop_unsupported_classifier_reasoning_effort: only a provider that explicitly rejects the
// param loses it, and the request's log body loses it with the live kwargs.
#[must_use]
pub fn drop_unsupported_classifier_reasoning_effort<S>(
    deployment: &DeploymentSpec,
    model: &str,
    kwargs: &mut JsonMap,
    support: &S,
) -> Option<String>
where
    S: ParamSupport,
{
    if absent_or_null(kwargs, REASONING_EFFORT) || !is_classifier_internal_call(kwargs) {
        return None;
    }
    if deployment_accepts_param(&deployment.to_json(), model, REASONING_EFFORT, support) {
        return None;
    }
    kwargs.pop(REASONING_EFFORT);
    if let Some(Json::Object(request)) = kwargs.get_mut("proxy_server_request")
        && let Some(Json::Object(body)) = request.get_mut("body")
    {
        body.pop(REASONING_EFFORT);
    }
    Some(format!(
        "litellm.router.py: dropping classifier reasoning_effort for model={model} because the selected deployment does not support it"
    ))
}

// _tier_params_the_target_accepts: a tier param no deployment behind the model declares would 400
// every call the tier routes, unless a deployment, the tier or the request allowlists it.
#[must_use]
pub fn tier_params_the_target_accepts<S>(
    model: &str,
    tier_params: &JsonMap,
    request_kwargs: &JsonMap,
    deployments: &[DeploymentSpec],
    support: &S,
) -> TierParams
where
    S: ParamSupport,
{
    if deployments.is_empty() {
        return TierParams {
            params: tier_params.clone(),
            dropped: BTreeSet::new(),
        };
    }
    let dropped = unsupported_params(model, tier_params, request_kwargs, deployments, support);
    if dropped.is_empty() {
        return TierParams {
            params: tier_params.clone(),
            dropped,
        };
    }
    tracing::warn!(
        "litellm.router.py: dropping tier params {} for model={model}, no deployment behind it declares them",
        dropped
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    );
    TierParams {
        params: tier_params
            .iter()
            .filter(|(key, _)| !dropped.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        dropped,
    }
}

fn unsupported_params<S>(
    model: &str,
    tier_params: &JsonMap,
    request_kwargs: &JsonMap,
    deployments: &[DeploymentSpec],
    support: &S,
) -> BTreeSet<String>
where
    S: ParamSupport,
{
    let allowlisted: BTreeSet<String> = declared_param_allowlist(tier_params)
        .union(&declared_param_allowlist(request_kwargs))
        .cloned()
        .collect();
    let candidates: BTreeSet<String> = provider_rejectable_params(&params_of(tier_params))
        .into_iter()
        .filter(|param| {
            !is_litellm_owned_kwarg(param)
                && !TIER_PARAMS_ALWAYS_KEPT.contains(&param.as_str())
                && !allowlisted.contains(param)
        })
        .collect();
    let deployment_jsons: Vec<JsonMap> = deployments.iter().map(DeploymentSpec::to_json).collect();
    candidates
        .into_iter()
        .filter(|param| {
            !deployment_jsons
                .iter()
                .any(|deployment| deployment_accepts_param(deployment, model, param, support))
        })
        .collect()
}

fn absent_or_null(map: &JsonMap, key: &str) -> bool {
    matches!(map.get(key), None | Some(Json::Null))
}

fn params_of(map: &JsonMap) -> Params {
    map.iter()
        .map(|(key, value)| (key.clone(), json_value(value)))
        .collect()
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(entries) => JsonValue::Array(entries.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), json_value(value)))
                .collect(),
        ),
    }
}
