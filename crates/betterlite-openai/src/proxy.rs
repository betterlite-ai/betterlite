pub mod error;
pub mod images;
pub mod models;
pub mod responses;

mod tests;

use betterlite_models::identity::provider_id::ProviderId;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::openai_base_config::{CatalogCapabilities, Object, OpenAiConfig};

use error::ProxyError;

pub const LITELLM_PROXY_PREFIX: &str = "litellm_proxy/";
// every name OPENAI_CHAT_COMPLETION_PARAMS adds to the inherited OpenAI list
const EXTRA_PARAMS: &str = "user request_timeout api_base api_version api_key deployment_id organization base_url default_headers timeout response_format reasoning_effort thinking include_server_side_tool_invocations verbosity";

#[derive(Debug, Clone, Default)]
pub struct ProxyOptions {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub use_litellm_proxy: bool,
}

#[derive(Debug)]
pub struct ProxyRoute {
    pub model: String,
    pub provider: ProviderId,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

impl ProxyOptions {
    // an explicit empty base falls through to the configured one, as LiteLLM's "a or b" chain does
    #[must_use]
    pub fn resolve_api_base(&self, api_base: Option<&str>) -> Option<String> {
        if let Some(explicit) = api_base.filter(|value| !value.is_empty()) {
            return Some(explicit.to_owned());
        }
        self.api_base
            .as_ref()
            .filter(|value| !value.is_empty())
            .cloned()
    }

    #[must_use]
    pub fn resolve_api_key(&self, api_key: Option<&SecretString>) -> Option<SecretString> {
        let explicit = api_key.filter(|key| !key.expose_secret().is_empty());
        let configured = self
            .api_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        explicit.or(configured).cloned()
    }

    // a deployment that sets the flag false does not veto a true environment or global value
    #[must_use]
    pub const fn should_use_litellm_proxy(&self, deployment_use_litellm_proxy: bool) -> bool {
        if deployment_use_litellm_proxy {
            return true;
        }
        self.use_litellm_proxy
    }

    // litellm_proxy_get_custom_llm_provider_info strips only the leading proxy prefix
    #[must_use]
    pub fn route(
        &self,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> ProxyRoute {
        let stripped = model.strip_prefix(LITELLM_PROXY_PREFIX).unwrap_or(model);
        ProxyRoute {
            model: stripped.to_owned(),
            provider: ProviderId::LitellmProxy,
            api_base: self.resolve_api_base(api_base),
            api_key: self.resolve_api_key(api_key),
        }
    }
}

// the proxy inherits the OpenAI list and accepts every OPENAI_CHAT_COMPLETION_PARAMS name on top
#[must_use]
pub fn supported_params(
    config: &OpenAiConfig,
    model: &str,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str> {
    let mut params = config.supported_params(model, catalog);
    params.extend(EXTRA_PARAMS.split_whitespace());
    params
}

pub fn map_params(
    config: &OpenAiConfig,
    model: &str,
    requested: &Object,
    existing: &mut Object,
    catalog: &CatalogCapabilities,
) -> Result<(), ProxyError> {
    let supported = supported_params(config, model, catalog);
    for (name, value) in requested {
        if name == "thinking" {
            continue;
        }
        if supported.contains(&name.as_str()) {
            existing.insert(name.clone(), value.clone());
        }
    }
    let Some(thinking) = requested.get("thinking") else {
        return Ok(());
    };
    match existing
        .entry(String::from("extra_body"))
        .or_insert_with(|| Value::Object(Object::new()))
    {
        Value::Object(fields) => {
            fields.insert(String::from("thinking"), thinking.clone());
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            return Err(ProxyError::ExtraBodyNotObject);
        }
    }
    Ok(())
}

// LiteLLM sends the model and messages untouched, then expands optional_params, which may replace either
#[must_use]
pub fn transform_request(model: &str, messages: Value, optional_params: &Object) -> Object {
    let mut body = Object::from_iter([
        (String::from("model"), Value::String(model.to_owned())),
        (String::from("messages"), messages),
    ]);
    body.extend(optional_params.clone());
    body
}
