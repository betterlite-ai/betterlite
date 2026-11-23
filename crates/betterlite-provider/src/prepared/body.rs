use std::io::Error as IoError;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::call_params::normalize_drop_params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::params::classification::RESERVED_PARAMETER_NAMES;
use betterlite_models::params::{
    CallType, ChatParameters, ParameterError, ParameterMapping, ParameterPolicy,
    map_listed_parameters,
};
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers as OpenAiHeaders, OpenAiConfig, OpenAiSettings,
};
use betterlite_paths::paths::delete::remove;
use betterlite_paths::paths::{Document, PathLimits};
use http::HeaderValue;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Error as JsonError;

use crate::operation::{OperationError, success};
use crate::prepared::deployment::Deployment;

// LiteLLM's DEFAULT_CHAT_COMPLETION_PARAM_VALUES: these go through the OpenAI parameter policy, the rest are forwarded
const KNOWN_PARAMS: &str = "functions function_call temperature top_p n stream stream_options stop max_tokens max_completion_tokens modalities prediction audio presence_penalty frequency_penalty logit_bias user model custom_llm_provider response_format seed tools tool_choice max_retries logprobs top_logprobs extra_headers api_version parallel_tool_calls drop_params allowed_openai_params additional_drop_params messages reasoning_effort verbosity thinking web_search_options include_server_side_tool_invocations service_tier safety_identifier prompt_cache_key prompt_cache_retention store metadata context_management";
// LiteLLM reads these call kwargs itself, so they never reach the provider body
const REMOVED_KEYS: [&str; 5] = [
    "model",
    "stream",
    "mock_response",
    "mock_tool_calls",
    "stream_chunk_size",
];
// get_non_default_completion_params drops LiteLLM's own kwargs before the check, and metadata is one of them
const OWNED_PARAMETER_NAMES: [&str; 1] = ["metadata"];
// the same bounds the responses side gives its drop paths (crates/betterlite-core/src/api/responses_parameters/nested.rs)
const DROP_PATH_LIMITS: PathLimits = PathLimits {
    max_path_bytes: 512,
    max_depth: 32,
    max_selectors: 32,
    max_visits: 4096,
};

pub(crate) struct ChatRequestFields {
    pub(crate) messages: JsonValue,
    pub(crate) params: IndexMap<String, JsonValue>,
}

pub(crate) fn chat_request_fields(
    request: &ChatRequest,
) -> Result<ChatRequestFields, OperationError> {
    let mut requested = request_fields(request, "chat")?;
    let messages = requested
        .shift_remove("messages")
        .unwrap_or(JsonValue::Array(Vec::new()));
    requested.retain(|key, _value| !REMOVED_KEYS.contains(&key.as_str()));
    Ok(ChatRequestFields {
        messages,
        params: requested,
    })
}

pub(crate) fn chat_fields(
    request: &ChatRequest,
    model: &str,
    catalog: &CatalogCapabilities,
    deployment: &Deployment,
) -> Result<IndexMap<String, JsonValue>, OperationError> {
    let ChatRequestFields { messages, params } = chat_request_fields(request)?;
    let policy = OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: OpenAiHeaders::new(),
        settings: OpenAiSettings::default(),
    };
    let supported = policy.supported_params(model, catalog);
    // _check_valid_arg runs before the body is built, so an unsupported name answers 400 here
    let mapping = listed_parameters(&params, &supported, model, deployment)?;
    let (drop_policy, _call_drop) = parameter_policy(&params, deployment)?;
    let (known, mut forwarded): (IndexMap<String, JsonValue>, IndexMap<String, JsonValue>) = params
        .into_iter()
        .partition(|(name, _value)| KNOWN_PARAMS.split(' ').any(|known| known == name));
    let mut optional: IndexMap<String, JsonValue> = known
        .into_iter()
        .filter(|(name, _value)| mapping.standard.contains_key(name))
        .collect();
    append_stream_options(request, &mut optional);
    let extra = provider_extra_body(&mut forwarded, &mapping, &drop_policy);
    optional.insert(String::from("extra_body"), JsonValue::Object(extra));
    drop_nested_paths(&mut optional, &drop_policy.additional_drop_params)?;
    let mut body: IndexMap<String, JsonValue> = IndexMap::new();
    body.insert(String::from("model"), JsonValue::Text(model.to_owned()));
    body.insert(String::from("messages"), messages);
    body.extend(optional);
    // the OpenAI client merges extra_body into the request, so its contents go out at the top level
    if let Some(JsonValue::Object(extra_fields)) = body.shift_remove("extra_body") {
        body.extend(extra_fields);
    }
    Ok(body)
}

// add_provider_specific_params_to_optional_params: the provider-specific params move under extra_body, filtered by model and the drop names, before the nested delete (utils.py L5065-L5079)
fn provider_extra_body(
    forwarded: &mut IndexMap<String, JsonValue>,
    mapping: &ParameterMapping,
    policy: &ParameterPolicy,
) -> IndexMap<String, JsonValue> {
    let drops = &policy.additional_drop_params;
    if drops.iter().any(|name| name == "extra_body") {
        return IndexMap::new();
    }
    let mut extra = IndexMap::new();
    if let Some(JsonValue::Object(base)) = forwarded.shift_remove("extra_body") {
        extra = base;
    }
    for (name, value) in forwarded.drain(..) {
        if value != JsonValue::Null && !mapping.dropped_fields.contains(&name) {
            extra.insert(name, value);
        }
    }
    extra.retain(|name, _value| name != "model" && !drops.contains(name));
    extra
}
// get_optional_params at the pin: the request's kwargs and the deployment's policy decide which names the provider gets.
pub(crate) fn listed_parameters(
    fields: &IndexMap<String, JsonValue>,
    supported: &[&'static str],
    model: &str,
    deployment: &Deployment,
) -> Result<ParameterMapping, OperationError> {
    let (mut policy, _call_drop) = parameter_policy(fields, deployment)?;
    // a nested path is not a parameter name: it stays out of the flat check and drop_nested_paths applies it later
    policy
        .additional_drop_params
        .retain(|name| !name.contains('.') && !name.contains('['));
    let parameters = chat_parameters(fields)?;
    map_listed_parameters(
        supported,
        model,
        CallType::ChatCompletion,
        &parameters,
        &policy,
    )
    .map_err(|source| OperationError::Parameters { source })
}

// get_optional_params deletes every nested additional_drop_params path from the mapped parameters last (utils.py L5035-L5041)
pub(crate) fn drop_nested_paths<T>(
    optional: &mut IndexMap<String, T>,
    additional_drop_params: &[String],
) -> Result<(), OperationError>
where
    T: Serialize + DeserializeOwned,
{
    let nested: Vec<&String> = additional_drop_params
        .iter()
        .filter(|path| path.contains('.') || path.contains('['))
        .collect();
    if nested.is_empty() {
        return Ok(());
    }
    let json =
        serde_json::to_value(&*optional).map_err(|source| OperationError::Encode { source })?;
    let mut document = Document { json };
    for path in nested {
        document = remove(&DROP_PATH_LIMITS, &document, path).map_err(|source| {
            OperationError::Parameters {
                source: ParameterError::Encode {
                    source: JsonError::io(IoError::other(format!(
                        "the drop path {path} cannot be applied: {source}"
                    ))),
                },
            }
        })?;
    }
    *optional =
        serde_json::from_value(document.json).map_err(|source| OperationError::Parameters {
            source: ParameterError::Encode { source },
        })?;
    Ok(())
}

// The router spreads the deployment's litellm_params under the request's kwargs, so a value the request sends wins.
pub(crate) fn parameter_policy(
    fields: &IndexMap<String, JsonValue>,
    deployment: &Deployment,
) -> Result<(ParameterPolicy, bool), OperationError> {
    let call_drop = match fields.get("drop_params") {
        None | Some(JsonValue::Null) => normalize_drop_params(deployment.drop_params.as_ref()),
        Some(value) => Some(convert(value)?),
    };
    let dropped = match fields.get("additional_drop_params") {
        None | Some(JsonValue::Null) => deployment.additional_drop_params.clone(),
        Some(value) => convert(value)?,
    };
    let allowed = match fields.get("allowed_openai_params") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(value) => convert(value)?,
    };
    let policy = ParameterPolicy {
        // _check_valid_arg reads the global flag and the call's flag as an OR
        drop_params: call_drop == Some(true) || deployment.global_drop_params,
        additional_drop_params: dropped,
        allowed_openai_params: allowed,
    };
    Ok((policy, call_drop == Some(true)))
}

fn chat_parameters(fields: &IndexMap<String, JsonValue>) -> Result<ChatParameters, OperationError> {
    let listed = fields
        .iter()
        .filter(|(name, _value)| {
            !RESERVED_PARAMETER_NAMES.contains(&name.as_str())
                && !OWNED_PARAMETER_NAMES.contains(&name.as_str())
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    convert(JsonValue::Object(listed))
}

pub(crate) fn convert<T, U>(value: T) -> Result<U, OperationError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let wire = serde_json::to_value(value).map_err(|source| OperationError::Encode { source })?;
    serde_json::from_value(wire).map_err(|source| OperationError::Parameters {
        source: ParameterError::Encode { source },
    })
}

pub(crate) fn encode(fields: IndexMap<String, JsonValue>) -> Result<Vec<u8>, OperationError> {
    serde_json::to_vec(&JsonValue::Object(fields))
        .map_err(|source| OperationError::Encode { source })
}

fn append_stream_options(request: &ChatRequest, optional: &mut IndexMap<String, JsonValue>) {
    if !request.stream {
        return;
    }
    optional.insert(String::from("stream"), JsonValue::Bool(true));
    if optional.contains_key("stream_options") {
        return;
    }
    let mut stream_options = IndexMap::new();
    stream_options.insert(String::from("include_usage"), JsonValue::Bool(true));
    optional.insert(
        String::from("stream_options"),
        JsonValue::Object(stream_options),
    );
}

pub(crate) fn request_fields<T>(
    request: &T,
    label: &'static str,
) -> Result<IndexMap<String, JsonValue>, OperationError>
where
    T: Serialize,
{
    let text =
        serde_json::to_string(request).map_err(|source| OperationError::Encode { source })?;
    let value: JsonValue =
        serde_json::from_str(&text).map_err(|source| OperationError::Encode { source })?;
    let JsonValue::Object(fields) = value else {
        return Err(OperationError::Encode {
            source: JsonError::io(IoError::other(format!(
                "a {label} request did not serialize as an object"
            ))),
        });
    };
    Ok(fields)
}

pub(crate) fn parse<T>(
    response: UpstreamResponse,
    operation: &'static str,
) -> Result<T, OperationError>
where
    T: DeserializeOwned,
{
    let checked = success(response)?;
    serde_json::from_slice(&checked.body)
        .map_err(|source| OperationError::Decode { operation, source })
}

pub(crate) fn header_value(text: &str, name: &str) -> Result<HeaderValue, OperationError> {
    HeaderValue::try_from(text).map_err(|source| OperationError::InvalidHeader {
        name: name.to_owned(),
        source,
    })
}

// The credential is marked sensitive so the Debug text of a prepared request never prints it
pub(crate) fn authorization_value(text: &str) -> Result<HeaderValue, OperationError> {
    let mut value = header_value(text, "authorization")?;
    value.set_sensitive(true);
    Ok(value)
}
