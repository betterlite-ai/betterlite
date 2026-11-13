use std::collections::BTreeMap;

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;

use crate::otel::presets::agentops::AgentopsPreset;
use crate::otel::presets::arize::{ArizePreset, arize_dynamic_headers};
use crate::otel::presets::base::Preset;
use crate::otel::presets::langfuse::{LangfusePreset, langfuse_dynamic_headers};
use crate::otel::presets::langtrace::LangtracePreset;
use crate::otel::presets::levo::LevoPreset;
use crate::otel::presets::newrelic::{
    NewrelicPreset, newrelic_dynamic_endpoint, newrelic_dynamic_headers,
};
use crate::otel::presets::phoenix::{PhoenixPreset, phoenix_project_headers};
use crate::otel::presets::signoz::{SignozPreset, signoz_dynamic_endpoint, signoz_dynamic_headers};
use crate::otel::presets::weave::{WeavePreset, weave_dynamic_headers};

pub mod agentops;
pub mod arize;
pub mod base;
pub mod destinations;
pub mod langfuse;
pub mod langtrace;
pub mod levo;
pub mod newrelic;
pub mod phoenix;
pub mod signoz;
pub mod utils;
pub mod weave;

#[must_use]
pub fn preset_for(callback_name: &str) -> Option<&'static dyn Preset> {
    match callback_name {
        "agentops" => Some(&AgentopsPreset),
        "arize" => Some(&ArizePreset),
        "arize_phoenix" => Some(&PhoenixPreset),
        "langfuse_otel" => Some(&LangfusePreset),
        "langtrace" => Some(&LangtracePreset),
        "levo" => Some(&LevoPreset),
        "newrelic" => Some(&NewrelicPreset),
        "signoz" => Some(&SignozPreset),
        "weave_otel" => Some(&WeavePreset),
        _ => None,
    }
}

#[must_use]
pub fn dynamic_otlp_headers(
    callback_name: Option<&str>,
    dynamic_params: &StandardCallbackDynamicParams,
    allowed_hosts: &[AllowedDestination],
) -> Option<BTreeMap<String, String>> {
    let parameters = dynamic_params_map(dynamic_params);
    let values = (!parameters.is_empty()).then_some(parameters)?;
    let headers = match callback_name {
        Some("arize") => arize_dynamic_headers(&values),
        Some("langfuse_otel") => langfuse_dynamic_headers(&values),
        Some("newrelic") => newrelic_dynamic_headers(&values),
        Some("signoz") => signoz_dynamic_headers(&values, allowed_hosts),
        Some("weave_otel") => weave_dynamic_headers(&values),
        Some(_) | None => return None,
    };
    (!headers.is_empty()).then_some(headers)
}

#[must_use]
pub fn dynamic_otlp_endpoint(
    callback_name: Option<&str>,
    dynamic_params: &StandardCallbackDynamicParams,
    allowed_hosts: &[AllowedDestination],
) -> Option<String> {
    let values = dynamic_params_map(dynamic_params);
    if values.is_empty() {
        return None;
    }
    if callback_name == Some("newrelic") {
        return Some(newrelic_dynamic_endpoint(&values));
    }
    if callback_name != Some("signoz") {
        return None;
    }
    signoz_dynamic_endpoint(&values, allowed_hosts)
}

// Project routing takes trusted key/team config, never the client-supplied dynamic parameters
#[must_use]
pub fn project_routing_headers(
    callback_name: Option<&str>,
    auth_metadata: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    if callback_name != Some("arize_phoenix") {
        return BTreeMap::new();
    }
    auth_metadata.map_or_else(BTreeMap::new, phoenix_project_headers)
}

// Only string values are routing inputs; anything else is dropped, and a non-object is no parameters at all
pub(crate) fn dynamic_params_map(
    params: &StandardCallbackDynamicParams,
) -> BTreeMap<String, String> {
    let Json::Object(fields) = params else {
        return BTreeMap::new();
    };
    fields
        .iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

mod tests;
