use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_config::sections::Section as _;
use betterlite_config::sections::anthropic_count_tokens::Raw as CountTokensRaw;
use betterlite_config::sections::anthropic_credentials::Settings as CredentialSettings;
use betterlite_config::types::LiteLlmParams;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Validate {
        #[serde(default)]
        cases: Vec<ValidateCase>,
    },
    Transform {
        #[serde(default)]
        cases: Vec<TransformCase>,
    },
    Headers {
        #[serde(default)]
        cases: Vec<HeaderCase>,
    },
    Handle {
        #[serde(default)]
        cases: Vec<HandleCase>,
    },
    Counter {
        #[serde(default)]
        cases: Vec<CounterCase>,
    },
    Factory {
        #[serde(default)]
        cases: Vec<FactoryCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidateCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Option<Value>,
    #[serde(default)]
    pub(crate) system: Option<Value>,
    #[serde(default)]
    pub(crate) tools: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Option<Value>,
    #[serde(default)]
    pub(crate) system: Option<Value>,
    #[serde(default)]
    pub(crate) tools: Option<Value>,
    #[serde(default)]
    pub(crate) optional_params: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderCase {
    pub(crate) api_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandleCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Option<Value>,
    pub(crate) api_key: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) request_timeout: Option<f64>,
    #[serde(default)]
    pub(crate) tools: Option<Value>,
    #[serde(default)]
    pub(crate) system: Option<Value>,
    #[serde(default)]
    pub(crate) optional_params: Option<serde_json::Map<String, Value>>,
    #[serde(default = "default_reply")]
    pub(crate) reply: Reply,
    #[serde(default)]
    pub(crate) failure: bool,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CounterCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Option<Value>,
    #[serde(default)]
    pub(crate) contents: Option<Value>,
    #[serde(default)]
    pub(crate) deployment: Option<Deployment>,
    #[serde(default)]
    pub(crate) request_model: Option<String>,
    #[serde(default)]
    pub(crate) tools: Option<Value>,
    #[serde(default)]
    pub(crate) system: Option<Value>,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default, rename = "module_api_key")]
    pub(crate) _module_api_key: Option<String>,
    #[serde(default, rename = "module_anthropic_key")]
    pub(crate) _module_anthropic_key: Option<String>,
    #[serde(default = "default_reply")]
    pub(crate) reply: Reply,
    #[serde(default)]
    pub(crate) failure: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FactoryCase {
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) providers: Vec<Option<String>>,
    #[serde(default)]
    pub(crate) messages: Option<Value>,
    #[serde(default)]
    pub(crate) deployment: Option<Deployment>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default = "default_reply")]
    pub(crate) reply: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Deployment {
    #[serde(default)]
    pub(crate) litellm_params: Option<DeploymentParams>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentParams {
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    pub(crate) body: String,
}

const fn default_status() -> u16 {
    200
}

fn default_reply() -> Reply {
    Reply {
        status: 200,
        body: String::from(r#"{"input_tokens":0}"#),
    }
}

pub(crate) fn json_value(value: &Value) -> Result<Json, Box<HelperError>> {
    serde_json::from_value(value.clone())
        .map_err(|error| failure(format!("the helper input is not valid JSON: {error}")))
}

pub(crate) fn optional_params(
    params: Option<&serde_json::Map<String, Value>>,
) -> Result<Option<IndexMap<String, Json>>, Box<HelperError>> {
    let Some(provided) = params else {
        return Ok(None);
    };
    let mut converted = IndexMap::new();
    for (name, value) in provided {
        converted.insert(name.clone(), json_value(value)?);
    }
    Ok(Some(converted))
}

pub(crate) fn deployment(input: Option<&Deployment>) -> Option<LiteLlmParams> {
    let params = input?.litellm_params.as_ref()?;
    Some(LiteLlmParams {
        ssl_verify: None,
        model: params.model.clone(),
        api_base: params.api_base.clone(),
        api_key: params.api_key.clone().map(SecretString::from),
        api_version: None,
        timeout: params.timeout,
        stream_timeout: None,
        max_retries: None,
        rpm: None,
        tpm: None,
        weight: None,
    })
}

pub(crate) fn credential_settings(
    environment: &BTreeMap<String, String>,
) -> Result<AnthropicSettings, Box<HelperError>> {
    let resolved = CredentialSettings::resolve_environment(environment).map_err(failure)?;
    Ok(AnthropicSettings {
        api_key: resolved.api_key,
        auth_token: resolved.auth_token,
        api_base: resolved.api_base,
        use_bearer_for_custom_base: resolved.use_bearer_for_custom_base,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    })
}

pub(crate) fn beta_version(
    environment: &BTreeMap<String, String>,
) -> Result<String, Box<HelperError>> {
    let raw = CountTokensRaw::default();
    let settings = raw.convert(environment).map_err(failure)?;
    Ok(settings.beta_version)
}

pub(crate) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "Error".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
