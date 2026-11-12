use super::invalid;
use crate::helpers::HelperError;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::local_servers::{
    ProviderEnvironment, docker_model_runner, lemonade_cost, llamafile, lm_studio, xinference_image,
};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServerProvider {
    DockerModelRunner,
    Llamafile,
    LmStudio,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChatProvider {
    DockerModelRunner,
    LmStudio,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCall {
    provider: ServerProvider,
    api_base: Option<String>,
    api_key: Option<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsCall {
    provider: ChatProvider,
    model: String,
    params: Vec<(String, Value)>,
    preset: Object,
    chat_models: BTreeSet<String>,
    text_models: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DockerCall {
    api_base: Option<String>,
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct XinferenceCall {
    model: String,
    params: Object,
    preset: Object,
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CostCall {
    model: String,
    prompt_tokens: u32,
    completion_tokens: u32,
}

impl ServerProvider {
    const fn environment_names(self) -> (&'static str, &'static str) {
        match self {
            Self::DockerModelRunner => (
                "DOCKER_MODEL_RUNNER_API_BASE",
                "DOCKER_MODEL_RUNNER_API_KEY",
            ),
            Self::Llamafile => ("LLAMAFILE_API_BASE", "LLAMAFILE_API_KEY"),
            Self::LmStudio => ("LM_STUDIO_API_BASE", "LM_STUDIO_API_KEY"),
        }
    }
}

pub(super) fn provider_value(call: &ProviderCall) -> Value {
    let (base_name, key_name) = call.provider.environment_names();
    let environment = ProviderEnvironment {
        api_base: call.env.get(base_name).cloned(),
        api_key: call.env.get(key_name).cloned().map(SecretString::from),
    };
    let key = call.api_key.clone().map(SecretString::from);
    let base = call.api_base.as_deref();
    let (api_base, api_key) = match call.provider {
        ServerProvider::DockerModelRunner => {
            let info = docker_model_runner::provider_info(base, key.as_ref(), &environment);
            (Some(info.api_base), info.api_key)
        }
        ServerProvider::Llamafile => {
            let info = llamafile::provider_info(base, key.as_ref(), &environment);
            (Some(info.api_base), info.api_key)
        }
        ServerProvider::LmStudio => {
            let info = lm_studio::provider_info(base, key.as_ref(), &environment);
            (info.api_base, info.api_key)
        }
    };
    json!({"api_base": api_base, "api_key": api_key.expose_secret()})
}

impl ParamsCall {
    fn config_parts(&self) -> (OpenAiConfig, CatalogCapabilities) {
        let base = OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        };
        let catalog = CatalogCapabilities {
            chat_models: self.chat_models.clone(),
            text_models: self.text_models.clone(),
        };
        (base, catalog)
    }
}

pub(super) fn params_value(call: &ParamsCall) -> Value {
    let (base, catalog) = call.config_parts();
    let mut optional = call.preset.clone();
    match call.provider {
        ChatProvider::DockerModelRunner => docker_model_runner::ChatConfig { base }
            .map_openai_params(
                &call.model,
                &call.params.iter().cloned().collect(),
                &mut optional,
                &catalog,
            ),
        ChatProvider::LmStudio => lm_studio::ChatConfig { base }.map_openai_params(
            &call.model,
            &call.params.iter().cloned().collect(),
            &mut optional,
            &catalog,
        ),
    }
    Value::Object(optional)
}

pub(super) fn docker_value(call: &DockerCall) -> Result<Value, Box<HelperError>> {
    let url = docker_model_runner::complete_url(call.api_base.as_deref());
    let transformed =
        docker_model_runner::transform_messages(&call.model, &call.messages, &RefuseDownloads);
    match transformed {
        Ok(messages) => {
            let wire =
                serde_json::to_value(&messages).map_err(|error| invalid(error.to_string()))?;
            Ok(json!({"url": url, "ok": true, "messages": wire}))
        }
        Err(_) => Ok(json!({"url": url, "ok": false})),
    }
}

pub(super) fn embed_value() -> Value {
    let config: Object = lm_studio::EMBEDDING_CONFIG_KEYS
        .iter()
        .map(|key| ((*key).to_owned(), Value::Null))
        .collect();
    json!({
        "supported": lm_studio::EMBEDDING_SUPPORTED_PARAMS,
        "config": config,
    })
}

pub(super) fn xinference_value(call: &XinferenceCall) -> Value {
    let mut optional = call.preset.clone();
    let mapped = xinference_image::map_openai_params(
        &call.model,
        &call.params,
        &mut optional,
        call.drop_params,
    );
    match mapped {
        Ok(()) => json!({
            "ok": true,
            "supported": xinference_image::SUPPORTED_PARAMS,
            "value": optional,
        }),
        Err(_) => json!({"ok": false, "supported": xinference_image::SUPPORTED_PARAMS}),
    }
}

pub(super) fn cost_value(call: &CostCall) -> Value {
    let (prompt_rate, completion_rate) = lemonade_cost::COST_PER_TOKEN;
    let prompt = prompt_rate.checked_mul(Decimal::from(call.prompt_tokens));
    let completion = completion_rate.checked_mul(Decimal::from(call.completion_tokens));
    json!({
        "model": call.model,
        "prompt_is_zero": prompt.is_some_and(|cost| cost.is_zero()),
        "completion_is_zero": completion.is_some_and(|cost| cost.is_zero()),
    })
}
