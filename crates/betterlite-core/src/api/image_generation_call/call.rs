use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use std::collections::BTreeMap;
use thiserror::Error;

use self::values::{header_map, string_list};

mod values;

// LiteLLM's timeout=600 default
pub const DEFAULT_TIMEOUT: f64 = 600.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomClientKind {
    Sync,
    Asynchronous,
}

// One call of litellm.image_generation: the declared arguments, the transport the caller already built and the rest
#[derive(Debug, Clone)]
pub struct ImageGenerationCall {
    pub model: Option<String>,
    pub prompt: String,
    pub n: Option<i64>,
    pub quality: Option<String>,
    pub response_format: Option<String>,
    pub size: Option<String>,
    pub style: Option<String>,
    pub user: Option<String>,
    pub timeout: f64,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub custom_llm_provider: Option<String>,
    // the client a caller passed, which only the custom branch reads, and only when its kind matches
    pub client_kind: Option<CustomClientKind>,
    pub now_seconds: i64,
    pub kwargs: IndexMap<String, Json>,
}

// The Azure token kwargs get_litellm_params copies into litellm_params_dict
#[derive(Debug, Clone, Default)]
pub struct LiteLlmCredentials {
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GenericParams {
    pub aimg_generation: bool,
    pub azure_ad_token_provider: bool,
    pub azure_ad_token: Option<Json>,
    pub mock_response: Option<String>,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub headers: BTreeMap<String, String>,
    pub base_model: Option<String>,
    pub organization: Option<String>,
    pub drop_params: Option<bool>,
    pub additional_drop_params: Vec<String>,
    pub credentials: LiteLlmCredentials,
}

impl GenericParams {
    pub fn read(kwargs: &IndexMap<String, Json>) -> Result<Self, CallError> {
        let mut params = Self::default();
        for (name, value) in kwargs {
            match name.as_str() {
                "aimg_generation" => {
                    params.aimg_generation =
                        optional_flag(value, "aimg_generation")?.unwrap_or(false)
                }
                "azure_ad_token_provider" => {
                    params.azure_ad_token_provider = !matches!(value, Json::Null);
                }
                // LiteLLM tests the value for truth later, so a nonstring is kept and falls back there
                "azure_ad_token" => params.azure_ad_token = Some(value.clone()),
                "mock_response" => params.mock_response = string(value, "mock_response")?,
                "extra_headers" => {
                    params.extra_headers = header_map(value, "extra_headers")?;
                }
                "headers" => params.headers = header_map(value, "headers")?.unwrap_or_default(),
                "base_model" => params.base_model = string(value, "base_model")?,
                "organization" => params.organization = string(value, "organization")?,
                "drop_params" => params.drop_params = optional_flag(value, "drop_params")?,
                "additional_drop_params" => {
                    params.additional_drop_params = string_list(value, "additional_drop_params")?;
                }
                "tenant_id" => params.credentials.tenant_id = string(value, "tenant_id")?,
                "client_id" => params.credentials.client_id = string(value, "client_id")?,
                "client_secret" => {
                    params.credentials.client_secret =
                        string(value, "client_secret")?.map(SecretString::from)
                }
                "azure_username" => {
                    params.credentials.azure_username = string(value, "azure_username")?
                }
                "azure_password" => {
                    params.credentials.azure_password =
                        string(value, "azure_password")?.map(SecretString::from)
                }
                "azure_scope" => params.credentials.azure_scope = string(value, "azure_scope")?,
                _ => {}
            }
        }
        Ok(params)
    }

    // `headers: kwargs.get("headers") or {}` then `headers.update(extra_headers)` for a non-None extra map
    #[must_use]
    pub fn merged_headers(&self) -> BTreeMap<String, String> {
        let mut headers = self.headers.clone();
        if let Some(extra) = &self.extra_headers {
            for (name, value) in extra {
                headers.insert(name.clone(), value.clone());
            }
        }
        headers
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CallError {
    #[error("{field} must be a string")]
    FieldText { field: &'static str },
    #[error("{field} must be an object of string values")]
    FieldHeaders { field: &'static str },
    #[error("{field} must be a boolean")]
    FieldFlag { field: &'static str },
    #[error("{field} must be a list of strings")]
    FieldList { field: &'static str },
}

fn string(value: &Json, field: &'static str) -> Result<Option<String>, CallError> {
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let Json::Text(text) = value else {
        return Err(CallError::FieldText { field });
    };
    Ok(Some(text.clone()))
}

const fn optional_flag(value: &Json, field: &'static str) -> Result<Option<bool>, CallError> {
    match value {
        Json::Bool(flag) => Ok(Some(*flag)),
        Json::Null => Ok(None),
        Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(CallError::FieldFlag { field })
        }
    }
}
