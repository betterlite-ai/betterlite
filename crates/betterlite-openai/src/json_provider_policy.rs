use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::openai_like_settings::Settings as OpenAiLikeSettings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    BaseClass, JSONProviderRegistry, ProviderConfig, ProviderDescriptor,
};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Message, convert_message_content_lists};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use crate::chat::ChatMessage;
use crate::openai_base_config::Headers;
use crate::response_input::error::ResponseInputError;
use crate::response_input::{ResponsesInput, transform_responses_request};

pub mod error;

use self::error::JsonProviderPolicyError;
mod tests;

// LiteLLM removes these from the parent's list when the model has no function calling
pub const TOOL_PARAMS: [&str; 5] = [
    "tools",
    "tool_choice",
    "function_call",
    "functions",
    "parallel_tool_calls",
];
const REASONING_EFFORT: &str = "reasoning_effort";
const CHAT_COMPLETIONS_SUFFIX: &str = "/chat/completions";
const RESPONSES_SUFFIX: &str = "/responses";

// openai_gpt selects B503/B668, every other base_class selects B1425's compatible policy
#[must_use]
pub fn base_class_parent(base_class: &BaseClass) -> ChatParent {
    if base_class.0.as_bytes() == b"openai_gpt" {
        ChatParent::Gpt
    } else {
        ChatParent::Compatible
    }
}

// explicit base, then the declared api_base_env, then base_url
fn resolve_descriptor_base(
    config: &ProviderConfig,
    explicit: Option<&str>,
    lookup: &dyn EnvironmentLookup,
) -> String {
    if let Some(base) = explicit.filter(|value| !value.is_empty()) {
        return base.to_owned();
    }
    let from_env = config
        .api_base_env
        .as_deref()
        .and_then(|name| lookup.lookup(name));
    match from_env {
        Some(base) if !base.is_empty() => base,
        _ => config.base_url.clone(),
    }
}

// an explicit key wins only while it is non-empty, then the declared api_key_env answers
fn resolve_descriptor_key(
    config: &ProviderConfig,
    explicit: Option<&SecretString>,
    lookup: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    explicit
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| {
            lookup
                .lookup(&config.api_key_env)
                .filter(|value| !value.is_empty())
                .map(SecretString::from)
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatParent {
    Gpt,
    Compatible,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChatCapabilities {
    pub supports_function_calling: bool,
    pub supports_reasoning: bool,
}

#[derive(Clone, Debug)]
pub struct ChatPolicy {
    pub descriptor: ProviderDescriptor,
}

impl ChatPolicy {
    // explicit base, then the declared api_base_env, then base_url; the key is explicit, then api_key_env
    #[must_use]
    pub fn resolve_provider_info(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
        lookup: &dyn EnvironmentLookup,
    ) -> (String, Option<SecretString>) {
        let config = &self.descriptor.config;
        let base = resolve_descriptor_base(config, api_base, lookup);
        let key = resolve_descriptor_key(config, api_key, lookup);
        (base, key)
    }

    // get_complete_url falls back straight to base_url and never touches the trailing slashes
    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>) -> String {
        let base = api_base
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.descriptor.config.base_url);
        if base.ends_with(CHAT_COMPLETIONS_SUFFIX) {
            base.to_owned()
        } else {
            format!("{base}{CHAT_COMPLETIONS_SUFFIX}")
        }
    }

    // the parent's list loses the tool params without function calling, and gains reasoning_effort when it has none
    #[must_use]
    pub fn supported_params(
        &self,
        parent_params: &[&str],
        capabilities: ChatCapabilities,
    ) -> Vec<String> {
        let mut params: Vec<String> = parent_params
            .iter()
            .map(|param| (*param).to_owned())
            .collect();
        if !capabilities.supports_function_calling {
            params.retain(|param| !TOOL_PARAMS.contains(&param.as_str()));
        }
        if capabilities.supports_reasoning && !params.iter().any(|param| param == REASONING_EFFORT)
        {
            params.push(REASONING_EFFORT.to_owned());
        }
        params
    }

    // a mapping is checked before the supported list, then the temperature constraints run in max, min, min_with_n order
    pub fn map_params(
        &self,
        non_default: &Object,
        existing: &mut Object,
        supported: &[String],
    ) -> Result<(), JsonProviderPolicyError> {
        let mappings = &self.descriptor.config.param_mappings;
        for (name, value) in non_default {
            if let Some(mapped) = mappings.get(name) {
                existing.insert(mapped.clone(), value.clone());
            } else if supported.iter().any(|param| param == name) {
                existing.insert(name.clone(), value.clone());
            }
        }
        self.clamp_temperature(existing)
    }

    fn clamp_temperature(&self, existing: &mut Object) -> Result<(), JsonProviderPolicyError> {
        let Some(temperature) = existing.get("temperature") else {
            return Ok(());
        };
        let constraints = &self.descriptor.config.constraints;
        if constraints.temperature_max.is_none()
            && constraints.temperature_min.is_none()
            && constraints.temperature_min_with_n_gt_1.is_none()
        {
            return Ok(());
        }
        // a constraint is a JSON number too, so it goes back through its decimal text to keep its scale
        let mut value = temperature.clone();
        if let Some(max) = constraints.temperature_max {
            let current = decimal_of(&value)?;
            if max < current {
                value = serde_json::from_str(&max.to_string()).unwrap_or(Json::Null);
            }
        }
        if let Some(min) = constraints.temperature_min {
            let current = decimal_of(&value)?;
            if min > current {
                value = serde_json::from_str(&min.to_string()).unwrap_or(Json::Null);
            }
        }
        if let Some(min_with_n) = constraints.temperature_min_with_n_gt_1 {
            let n = match existing.get("n") {
                None => Decimal::ONE,
                Some(Json::Null) => return Err(JsonProviderPolicyError::NNotANumber),
                Some(value) => {
                    decimal_of(value).map_err(|_| JsonProviderPolicyError::NNotANumber)?
                }
            };
            if n > Decimal::ONE {
                let current = decimal_of(&value)?;
                if current < min_with_n {
                    value = serde_json::from_str(&min_with_n.to_string()).unwrap_or(Json::Null);
                }
            }
        }
        existing.insert("temperature".to_owned(), value);
        Ok(())
    }

    // LiteLLM flattens a content list into a string before the parent's message conversion
    pub fn transform_messages(
        &self,
        messages: &[ChatMessage],
    ) -> Result<Vec<ChatMessage>, JsonProviderPolicyError> {
        if !self.converts_content_lists() {
            return Ok(messages.to_vec());
        }
        let value = serde_json::to_value(messages)
            .map_err(|source| JsonProviderPolicyError::Message { source })?;
        let mut prompt: Vec<Message> = serde_json::from_value(value)
            .map_err(|source| JsonProviderPolicyError::Message { source })?;
        let collapsed = convert_message_content_lists(&mut prompt);
        let value = serde_json::to_value(collapsed)
            .map_err(|source| JsonProviderPolicyError::Message { source })?;
        serde_json::from_value(value).map_err(|source| JsonProviderPolicyError::Message { source })
    }

    #[must_use]
    pub fn converts_content_lists(&self) -> bool {
        self.descriptor
            .config
            .special_handling
            .get("convert_content_list_to_string")
            .copied()
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug)]
pub struct ResponsesPolicy {
    pub descriptor: ProviderDescriptor,
}

impl ResponsesPolicy {
    // every trailing slash goes, and /responses is always appended, even when the base already ends with it
    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>, lookup: &dyn EnvironmentLookup) -> String {
        format!(
            "{}{RESPONSES_SUFFIX}",
            resolve_descriptor_base(&self.descriptor.config, api_base, lookup)
                .trim_end_matches('/')
        )
    }

    // only a truthy key writes Authorization, and the caller's other headers stay as they are
    #[must_use]
    pub fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        lookup: &dyn EnvironmentLookup,
    ) -> Headers {
        match resolve_descriptor_key(&self.descriptor.config, api_key, lookup) {
            Some(key) => authorized_headers(headers, Some(&key)),
            None => headers,
        }
    }

    // force_store_false overwrites the caller's store value, then B508 builds the wire body
    pub fn transform_request(
        &self,
        model: &str,
        input: &ResponsesInput,
        params: &Object,
        model_info: Option<&Object>,
    ) -> Result<Object, ResponseInputError> {
        let mut params = params.clone();
        if self.force_store_false() {
            params.insert("store".to_owned(), Json::Bool(false));
        }
        transform_responses_request(model, input, &params, self.descriptor.provider, model_info)
    }

    #[must_use]
    pub fn force_store_false(&self) -> bool {
        self.descriptor
            .config
            .special_handling
            .get("force_store_false")
            .copied()
            .unwrap_or(false)
    }
}

// a truthy Bearer key overwrites Authorization, and every other caller header stays as it is
fn authorized_headers(mut headers: Headers, key: Option<&SecretString>) -> Headers {
    if let Some(key) = key {
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    headers
}

// LiteLLM caches the generated Responses class by slug for the life of the process, so the first descriptor wins
#[derive(Debug, Default)]
pub struct ResponsesPolicyCache {
    pub policies: BTreeMap<ProviderId, ResponsesPolicy>,
}

impl ResponsesPolicyCache {
    pub fn policy(
        &mut self,
        registry: &JSONProviderRegistry,
        provider: ProviderId,
    ) -> Option<&ResponsesPolicy> {
        if !self.policies.contains_key(&provider) {
            let descriptor = registry.providers.get(&provider)?.clone();
            self.policies
                .insert(provider, ResponsesPolicy { descriptor });
        }
        self.policies.get(&provider)
    }

    // descriptors from provider code, such as EdenAI's, are cached under the same finite identity space
    pub fn policy_for_descriptor(&mut self, descriptor: ProviderDescriptor) -> &ResponsesPolicy {
        let provider = descriptor.provider;
        self.policies
            .entry(provider)
            .or_insert_with(|| ResponsesPolicy { descriptor })
    }
}

// OpenAILikeResponsesConfig's generic identity over the OPENAI_LIKE_API_KEY/API_BASE settings
#[derive(Clone, Debug)]
pub struct OpenAiLikeResponsesPolicy {
    pub settings: OpenAiLikeSettings,
}

impl OpenAiLikeResponsesPolicy {
    // the base must come from the caller or OPENAI_LIKE_API_BASE; there is no default endpoint
    pub fn complete_url(&self, api_base: Option<&str>) -> Result<String, JsonProviderPolicyError> {
        match self.settings.responses_base_required(api_base) {
            Some(base) => Ok(format!("{}{RESPONSES_SUFFIX}", base.trim_end_matches('/'))),
            None => Err(JsonProviderPolicyError::OpenAiLikeBaseRequired),
        }
    }

    #[must_use]
    pub fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
    ) -> Headers {
        let fallback = self.settings.api_key.as_ref();
        if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
            return authorized_headers(headers, Some(key));
        }
        authorized_headers(headers, fallback)
    }
}

fn decimal_of(value: &Json) -> Result<Decimal, JsonProviderPolicyError> {
    match value {
        Json::Number(number) => number
            .to_string()
            .parse()
            .map_err(|_| JsonProviderPolicyError::TemperatureNotANumber),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(JsonProviderPolicyError::TemperatureNotANumber)
        }
    }
}
