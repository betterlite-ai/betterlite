use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Formatter};

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;
pub type Object = Map<String, Value>;
pub type Headers = BTreeMap<String, String>;

pub mod forwarding;
use forwarding::BaseConfigError;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogCapabilities {
    pub chat_models: BTreeSet<String>,
    pub text_models: BTreeSet<String>,
}
impl CatalogCapabilities {
    #[must_use]
    pub const fn new(chat_models: BTreeSet<String>, text_models: BTreeSet<String>) -> Self {
        Self {
            chat_models,
            text_models,
        }
    }

    #[must_use]
    pub fn is_catalog_model(&self, model: &str) -> bool {
        let checked = model
            .split_once("responses/")
            .map_or(model, |(_, suffix)| suffix);
        self.chat_models.contains(checked) || self.text_models.contains(checked)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFamily {
    OpenAi,
    AzureAiStudio,
    Other,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    OSeries,
    GptReasoning,
    Audio,
    Gpt,
    Unknown,
}
#[derive(Clone)]
pub struct OpenAiConfig {
    family: ConfigFamily,
    api_key: Option<SecretString>,
    headers: Headers,
    settings: Object,
}
impl fmt::Debug for OpenAiConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiConfig")
            .field("family", &self.family)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("headers", &self.headers)
            .field("settings", &self.settings)
            .finish()
    }
}
impl OpenAiConfig {
    #[must_use]
    pub const fn new(
        family: ConfigFamily,
        api_key: Option<SecretString>,
        headers: Headers,
        settings: Object,
    ) -> Self {
        Self {
            family,
            api_key,
            headers,
            settings,
        }
    }

    #[must_use]
    pub fn get_config(&self) -> Object {
        self.settings.clone()
    }

    #[must_use]
    pub const fn family(&self) -> ConfigFamily {
        self.family
    }
    #[must_use]
    pub fn validate_environment(&self) -> Headers {
        let authorization = self.api_key.as_ref().map_or_else(
            || "Bearer None".to_owned(),
            |key| format!("Bearer {}", key.expose_secret()),
        );
        let mut headers = BTreeMap::from([(String::from("Authorization"), authorization)]);
        headers.extend(self.headers.clone());
        headers
    }
    #[must_use]
    pub fn supported_params(
        &self,
        model: &str,
        catalog: &CatalogCapabilities,
    ) -> Vec<&'static str> {
        let mut params: Vec<&'static str> = BASE_PARAMS.split_whitespace().collect();
        if model != "gpt-3.5-turbo-16k" && model != "gpt-4" {
            params.push("response_format");
        }
        if catalog.is_catalog_model(model) {
            params.push("user");
        }
        if matches!(self.family, ConfigFamily::OpenAi) && !catalog.is_catalog_model(model) {
            params.push("reasoning_effort");
        }
        params
    }

    pub fn map_params(
        &self,
        model: &str,
        requested: &Object,
        existing: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        let supported = self.supported_params(model, catalog);
        for (name, value) in requested {
            if supported.contains(&name.as_str()) {
                existing.insert(name.clone(), value.clone());
            }
        }
    }

    #[must_use]
    pub fn request_body(&self, model: &str, messages: Value, optional: &Object) -> Object {
        let mut body = Object::from_iter([
            (String::from("model"), Value::String(model.to_owned())),
            (String::from("messages"), messages),
        ]);
        body.extend(optional.clone());
        body
    }
}
#[must_use]
pub fn model_family(model: &str, catalog: &CatalogCapabilities) -> ModelFamily {
    if is_o_series_model(model, catalog) {
        ModelFamily::OSeries
    } else if is_gpt_reasoning_series_name(model) {
        ModelFamily::GptReasoning
    } else if is_gpt_audio_model(model, catalog) {
        ModelFamily::Audio
    } else if catalog.is_catalog_model(model) {
        ModelFamily::Gpt
    } else {
        ModelFamily::Unknown
    }
}

pub fn is_o_series_model(model: &str, catalog: &CatalogCapabilities) -> bool {
    let name = model.rsplit('/').next().unwrap_or(model);
    name.len() > 1
        && name.starts_with('o')
        && name.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
        && catalog.chat_models.contains(name)
}

#[must_use]
pub fn is_gpt_reasoning_series_name(model: &str) -> bool {
    let final_component = model.rsplit('/').next().unwrap_or(model);
    (model.contains("gpt-5") || model.contains("gpt-6")) && !final_component.contains("gpt-5-chat")
}

#[must_use]
pub fn is_gpt_audio_model(model: &str, catalog: &CatalogCapabilities) -> bool {
    let in_catalog = catalog.is_catalog_model(model);
    in_catalog && model.contains("audio")
}

#[must_use]
pub fn is_azure_gpt5_model(model: &str) -> bool {
    let reasoning = is_gpt_reasoning_series_name(model);
    reasoning || model.contains("gpt5_series")
}

#[must_use]
pub fn metadata_filter_limit(metadata: Option<&Object>) -> Option<BTreeMap<String, String>> {
    let values = metadata?;
    Some(
        values
            .iter()
            .filter(|(key, value)| key.as_str() != "hidden_params" && value.is_string())
            .take(16)
            .filter_map(|(key, value)| value.as_str().map(|text| (key.clone(), text.to_owned())))
            .collect(),
    )
}

pub fn overlapping_api_base_path(
    api_base: &str,
    ending_path: &str,
) -> Result<String, BaseConfigError> {
    let mut url = Url::parse(api_base).map_err(BaseConfigError::InvalidUrl)?;
    let base: Vec<&str> = url
        .path()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let ending: Vec<&str> = ending_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let mut path = None;
    for index in 0..=base.len() {
        let overlap = base.len().saturating_sub(index);
        if base
            .get(index..)
            .is_some_and(|suffix| suffix == ending.get(..overlap).unwrap_or(&[]))
        {
            path = Some(
                base.get(..index)
                    .unwrap_or(&[])
                    .iter()
                    .chain(ending.iter())
                    .copied()
                    .collect::<Vec<_>>(),
            );
            break;
        }
    }
    let segments = path.unwrap_or_else(|| base.iter().chain(ending.iter()).copied().collect());
    url.set_path(&format!("/{}", segments.join("/")));
    Ok(url.to_string())
}

const BASE_PARAMS: &str = "frequency_penalty logit_bias logprobs top_logprobs max_tokens max_completion_tokens modalities prediction n presence_penalty seed stop stream stream_options temperature top_p tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store";

mod tests;
