use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_openai::chat::ChatMessage;
use betterlite_prompts::arguments::json::{Json, Object};
use http::HeaderMap;
use secrecy::ExposeSecret as _;

use crate::oci_chat_wire::types::oci::OciVendor;
use crate::oci_common::OciError;
use crate::oci_common::base_url::{OCI_API_VERSION, get_oci_base_url};
use crate::oci_common::credentials::OciCredentials;
use crate::oci_common::environment::{OciEnvironment, validate_oci_environment};

const EMPTY_MESSAGES: &str =
    "kwarg `messages` must be an array of messages that follow the openai chat standard";

const MISSING_CREDENTIALS: &str = "Missing required parameters: oci_user, oci_fingerprint, oci_tenancy, oci_compartment_id and at least one of oci_key or oci_key_file. These can be supplied via optional_params or via OCI_USER, OCI_FINGERPRINT, OCI_TENANCY, OCI_COMPARTMENT_ID, OCI_KEY_FILE environment variables. Alternatively, provide an oci_signer object from the OCI SDK.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OciParam {
    Oci(&'static str),
    Unsupported,
}

// LiteLLM builds the Cohere map from the GENERIC one; this table keeps its order
const GENERIC_PARAMS: [(&str, OciParam); 27] = [
    ("stream", OciParam::Oci("isStream")),
    ("max_tokens", OciParam::Oci("maxTokens")),
    ("max_completion_tokens", OciParam::Oci("maxTokens")),
    ("temperature", OciParam::Oci("temperature")),
    ("tools", OciParam::Oci("tools")),
    ("frequency_penalty", OciParam::Oci("frequencyPenalty")),
    ("logprobs", OciParam::Oci("logProbs")),
    ("logit_bias", OciParam::Oci("logitBias")),
    ("n", OciParam::Oci("numGenerations")),
    ("presence_penalty", OciParam::Oci("presencePenalty")),
    ("reasoning_effort", OciParam::Oci("reasoningEffort")),
    ("seed", OciParam::Oci("seed")),
    ("stop", OciParam::Oci("stop")),
    ("tool_choice", OciParam::Oci("toolChoice")),
    ("top_p", OciParam::Oci("topP")),
    ("max_retries", OciParam::Unsupported),
    ("top_logprobs", OciParam::Unsupported),
    ("modalities", OciParam::Unsupported),
    ("prediction", OciParam::Unsupported),
    ("stream_options", OciParam::Unsupported),
    ("function_call", OciParam::Unsupported),
    ("functions", OciParam::Unsupported),
    ("extra_headers", OciParam::Unsupported),
    ("parallel_tool_calls", OciParam::Unsupported),
    ("audio", OciParam::Unsupported),
    ("web_search_options", OciParam::Unsupported),
    ("response_format", OciParam::Oci("responseFormat")),
];

const COHERE_UNSUPPORTED: [&str; 5] = [
    "tool_choice",
    "n",
    "reasoning_effort",
    "logprobs",
    "logit_bias",
];

pub(crate) fn param_map(vendor: OciVendor) -> Vec<(&'static str, OciParam)> {
    match vendor {
        OciVendor::Generic => GENERIC_PARAMS.to_vec(),
        OciVendor::Cohere => cohere_params(),
    }
}

fn cohere_params() -> Vec<(&'static str, OciParam)> {
    let mut params: Vec<_> = GENERIC_PARAMS
        .iter()
        .map(|&(key, alias)| match key {
            "stop" => (key, OciParam::Oci("stopSequences")),
            _ if COHERE_UNSUPPORTED.contains(&key) => (key, OciParam::Unsupported),
            _ => (key, alias),
        })
        .collect();
    params.push(("top_k", OciParam::Oci("topK")));
    params
}

#[must_use]
pub fn get_supported_openai_params(model: &str) -> Vec<&'static str> {
    param_map(get_vendor_from_model(model))
        .into_iter()
        .filter(|(_, alias)| matches!(alias, OciParam::Oci(_)))
        .map(|(key, _)| key)
        .collect()
}

#[must_use]
pub fn get_vendor_from_model(model: &str) -> OciVendor {
    let name = strip_oci_prefix(model);
    let vendor = name.split('.').next().unwrap_or_default().to_lowercase();
    if vendor == "cohere" {
        OciVendor::Cohere
    } else {
        OciVendor::Generic
    }
}

fn strip_oci_prefix(model: &str) -> &str {
    match model.get(..4) {
        Some(prefix) if prefix.eq_ignore_ascii_case("oci/") => model.get(4..).unwrap_or(model),
        _ => model,
    }
}

pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: &Object,
    model: &str,
    drop_params: bool,
) -> Result<Object, OciError> {
    let params = param_map(get_vendor_from_model(model));
    let mut merged = non_default_params.clone();
    merged.extend(optional_params.clone());
    let mut adapted = Object::new();
    for (key, value) in merged {
        let alias = params
            .iter()
            .find(|(name, _)| *name == key.as_str())
            .map(|(_, alias)| *alias);
        match alias {
            Some(OciParam::Unsupported) => {
                if key == "max_retries" || (key == "n" && n_is_one(&value)) || drop_params {
                    continue;
                }
                return Err(OciError {
                    status: 400,
                    message: format!("param `{key}` is not supported on OCI"),
                });
            }
            Some(OciParam::Oci(oci_name)) => {
                adapted.insert(oci_name.to_owned(), value.clone());
                if oci_name == "responseFormat" {
                    adapted.insert(String::from("response_format"), value);
                }
            }
            None => {
                adapted.insert(key, value);
            }
        }
    }
    Ok(adapted)
}

fn n_is_one(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Number(number) => number.as_f64() == Some(1.0_f64),
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
    }
}

pub fn validate_environment(
    headers: &mut HeaderMap,
    messages: &[ChatMessage],
    credentials: &OciCredentials,
    environment: &OciEnvironment,
) -> Result<(), OciError> {
    if messages.is_empty() {
        return Err(OciError {
            status: 400,
            message: EMPTY_MESSAGES.to_owned(),
        });
    }
    let present = |value: Option<&str>| value.is_some_and(|text| !text.is_empty());
    let key_present = credentials
        .oci_key
        .as_ref()
        .is_some_and(|key| !key.expose_secret().is_empty())
        || present(credentials.oci_key_file.as_deref());
    let complete = present(credentials.oci_user.as_deref())
        && present(credentials.oci_fingerprint.as_deref())
        && present(credentials.oci_tenancy.as_deref())
        && present(credentials.oci_compartment_id.as_deref())
        && key_present;
    if !complete {
        return Err(OciError {
            status: 401,
            message: MISSING_CREDENTIALS.to_owned(),
        });
    }
    validate_oci_environment(headers, environment)
}

pub fn get_complete_url(
    credentials: &OciCredentials,
    api_base: Option<&str>,
) -> Result<String, OciError> {
    Ok(format!(
        "{}/{OCI_API_VERSION}/actions/chat",
        get_oci_base_url(&credentials.oci_region, api_base)?
    ))
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16) -> OciError {
    OciError {
        status: status_code,
        message: message.to_owned(),
    }
}

#[must_use]
pub async fn uses_max_completion_tokens<Predicates: CapabilityPredicates + Send>(
    model: &str,
    predicates: &mut Predicates,
) -> bool {
    if model.is_empty() {
        return false;
    }
    let name = strip_oci_prefix(model);
    let lowered = name.to_lowercase();
    if lowered.starts_with("openai.") {
        return !lowered.starts_with("openai.gpt-oss");
    }
    predicates
        .supports_factory(
            &CapabilityRequest {
                model: name,
                custom_llm_provider: Some("oci"),
            },
            NamedCapability::Reasoning.key(),
        )
        .await
}
