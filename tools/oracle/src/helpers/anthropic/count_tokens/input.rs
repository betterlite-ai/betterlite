use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_anthropic::count_tokens::wire::CountTokensBody;
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

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Validate,
    Transform,
    Headers,
    Handle,
    Counter,
    Factory,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) cases: Vec<Case>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Case {
    pub(crate) model: String,
    pub(crate) messages: Option<Value>,
    pub(crate) contents: Option<Value>,
    pub(crate) system: Option<Value>,
    pub(crate) tools: Option<Value>,
    pub(crate) optional_params: Option<serde_json::Map<String, Value>>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) timeout: Option<f64>,
    pub(crate) request_timeout: Option<f64>,
    pub(crate) deployment: Option<Deployment>,
    pub(crate) request_model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(rename = "module_api_key")]
    pub(crate) _module_api_key: Option<String>,
    #[serde(rename = "module_anthropic_key")]
    pub(crate) _module_anthropic_key: Option<String>,
    pub(crate) reply: Option<Reply>,
    pub(crate) models: Vec<String>,
    pub(crate) providers: Vec<Option<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Deployment {
    pub(crate) litellm_params: Option<DeploymentParams>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DeploymentParams {
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) timeout: Option<f64>,
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

pub(crate) fn json_value(value: &Value) -> Result<Json, Box<HelperError>> {
    serde_json::from_value(value.clone())
        .map_err(|error| failure(format!("the helper input is not valid JSON: {error}")))
}

pub(crate) struct Parts {
    messages: Json,
    system: Option<Json>,
    tools: Option<Json>,
    params: IndexMap<String, Json>,
}

impl Parts {
    pub(crate) fn of(case: &Case) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            messages: case.messages.as_ref().map_or(Ok(Json::Null), json_value)?,
            system: case.system.as_ref().map(json_value).transpose()?,
            tools: case.tools.as_ref().map(json_value).transpose()?,
            params: optional_params(case.optional_params.as_ref())?,
        })
    }

    pub(crate) const fn body<'case>(&'case self, model: &'case str) -> CountTokensBody<'case> {
        CountTokensBody {
            model,
            messages: &self.messages,
            tools: self.tools.as_ref(),
            system: self.system.as_ref(),
            optional_params: &self.params,
        }
    }
}

fn optional_params(
    params: Option<&serde_json::Map<String, Value>>,
) -> Result<IndexMap<String, Json>, Box<HelperError>> {
    let mut converted = IndexMap::new();
    for (name, value) in params.into_iter().flatten() {
        converted.insert(name.clone(), json_value(value)?);
    }
    Ok(converted)
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
