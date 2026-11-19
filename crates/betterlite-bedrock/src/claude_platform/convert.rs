use betterlite_config::sections::claude_platform_inputs::{PlatformValue, UnsupportedParams};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::claude_platform_inputs::OVERRIDE_PARAMS_KEY;
use crate::claude_platform_inputs::body::{
    PlatformWarning, UnsupportedResolution, filter_request_body, resolve_unsupported_override,
};
use crate::model_routes::ParamMap;

// serde already knows each JSON-shaped record, so one encoded round trip moves a map between them in order.
pub fn mapped<Source, Target>(
    params: &IndexMap<String, Source>,
) -> Result<IndexMap<String, Target>, serde_json::Error>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    params
        .iter()
        .map(|(key, value)| {
            let converted: Target = serde_json::from_value(serde_json::to_value(value)?)?;
            Ok((key.clone(), converted))
        })
        .collect()
}

pub fn prompt_messages(messages: &[Json]) -> Result<Vec<PromptJson>, serde_json::Error> {
    messages
        .iter()
        .map(|message| serde_json::from_value::<PromptJson>(serde_json::to_value(message)?))
        .collect()
}

pub fn contract_params(params: &IndexMap<String, PromptJson>) -> IndexMap<String, Json> {
    params
        .iter()
        .map(|(key, value)| (key.clone(), Json::from(value)))
        .collect()
}

// LiteLLM accepts a list, set, frozenset or tuple of names and warns about every other JSON shape.
fn unsupported(value: &Json) -> Result<UnsupportedParams, serde_json::Error> {
    match value {
        Json::List(_) => {
            let items: Vec<PlatformValue> = serde_json::from_value(serde_json::to_value(value)?)?;
            Ok(UnsupportedParams::List(items))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            let other: PlatformValue = serde_json::from_value(serde_json::to_value(value)?)?;
            Ok(UnsupportedParams::Other(other))
        }
    }
}

// The request's own override wins; a null in it reads as absent and exposes the litellm_params value.
pub fn resolve_override(
    optional_params: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Json>,
    log_invalid: bool,
) -> Result<UnsupportedResolution, serde_json::Error> {
    let optional = optional_params
        .get(OVERRIDE_PARAMS_KEY)
        .map(unsupported)
        .transpose()?;
    let deployment = litellm_params
        .get(OVERRIDE_PARAMS_KEY)
        .map(unsupported)
        .transpose()?;
    Ok(resolve_unsupported_override(
        deployment.as_ref(),
        optional.as_ref(),
        log_invalid,
    ))
}

// Every warning the policy emitted, in the order LiteLLM logs them: the override first, then the drop.
pub fn filtered_params(
    optional_params: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Json>,
    log_invalid: bool,
    log_dropped: bool,
) -> Result<(ParamMap, Vec<PlatformWarning>), serde_json::Error> {
    let resolution = resolve_override(optional_params, litellm_params, log_invalid)?;
    let converted: ParamMap = mapped(optional_params)?;
    let filtered = filter_request_body(&converted, resolution.names.as_ref(), log_dropped);
    let mut warnings = Vec::new();
    warnings.extend(resolution.warning);
    warnings.extend(filtered.warning);
    Ok((filtered.body, warnings))
}
