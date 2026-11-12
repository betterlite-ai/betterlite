use std::collections::BTreeMap;

use betterlite_http::url_path::encode_segment;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::{FLOW_ID_IN_PARAMS_MESSAGE, FLOW_ID_REQUIRED_MESSAGE, LangFlowError};

pub const API_BASE_ENV: &str = "LANGFLOW_API_BASE";
pub const API_KEY_ENV: &str = "LANGFLOW_API_KEY";
pub const DEFAULT_API_BASE: &str = "http://localhost:7860";
pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["stream"];
// LiteLLM's supports_stream_param_in_request_body is false: the run API answers with one JSON body
pub const STREAM_PARAM_IN_BODY: bool = false;

const API_KEY_HEADER: &str = "x-api-key";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const FLOW_ID_FIELD: &str = "flow_id";
const JSON_CONTENT_TYPE: &str = "application/json";
const RUN_PATH: &str = "/api/v1/run/";

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub api_base: String,
    pub api_key: Option<SecretString>,
}

pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderInfo, LangFlowError> {
    let base = match api_base.filter(|base| !base.is_empty()) {
        Some(base) => String::from(base),
        None => secret_text(secrets, environment, API_BASE_ENV)?
            .unwrap_or_else(|| String::from(DEFAULT_API_BASE)),
    };
    let key = match api_key.filter(|key| !key.expose_secret().is_empty()) {
        Some(key) => Some(key.clone()),
        None => secret_text(secrets, environment, API_KEY_ENV)?.map(SecretString::from),
    };
    Ok(ProviderInfo {
        api_base: base,
        api_key: key,
    })
}

fn secret_text(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<String>, LangFlowError> {
    let value = secrets
        .get_secret_str(environment, name, None)
        .map_err(|source| LangFlowError::Environment { name, source })?;
    Ok(value
        .filter(|text| !text.expose_secret().is_empty())
        .map(|text| text.expose_secret().to_owned()))
}

#[must_use]
pub const fn map_openai_params(
    _non_default_params: &Object,
    optional_params: Object,
    _model: &str,
    _drop_params: bool,
) -> Object {
    // LiteLLM's LangFlow map hands optional_params back untouched, so only the stream flag is accepted
    optional_params
}

pub fn flow_id(model: &str, optional_params: &Object) -> Result<String, LangFlowError> {
    if optional_params
        .get(FLOW_ID_FIELD)
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return Err(LangFlowError::Api {
            status_code: 400,
            message: String::from(FLOW_ID_IN_PARAMS_MESSAGE),
        });
    }
    let id = model
        .split_once('/')
        .map_or(model, |(_provider, rest)| rest)
        .trim();
    if id.is_empty() {
        return Err(LangFlowError::Api {
            status_code: 400,
            message: String::from(FLOW_ID_REQUIRED_MESSAGE),
        });
    }
    Ok(id.to_owned())
}

pub fn complete_url(
    api_base: Option<&str>,
    model: &str,
    optional_params: &Object,
) -> Result<String, LangFlowError> {
    let Some(base) = api_base else {
        return Err(LangFlowError::ApiBaseRequired);
    };
    let id = flow_id(model, optional_params)?;
    let encoded = encode_segment(&id, FLOW_ID_FIELD).map_err(|source| LangFlowError::FlowId {
        flow_id: id,
        source,
    })?;
    Ok(format!("{}{RUN_PATH}{encoded}", base.trim_end_matches('/')))
}

pub fn validate_environment(
    headers: &mut BTreeMap<String, String>,
    api_key: Option<&SecretString>,
) {
    headers.insert(
        String::from(CONTENT_TYPE_HEADER),
        String::from(JSON_CONTENT_TYPE),
    );
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        headers.insert(String::from(API_KEY_HEADER), key.expose_secret().to_owned());
    }
}

pub fn sign_request(
    headers: BTreeMap<String, String>,
    request_data: &Object,
) -> Result<BTreeMap<String, String>, LangFlowError> {
    super::request::reject_tweaks(request_data)?;
    Ok(headers)
}
