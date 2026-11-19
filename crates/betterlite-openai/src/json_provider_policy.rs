use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::json_provider_registry::{BaseClass, ProviderConfig, ProviderDescriptor};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Message, convert_message_content_lists};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use crate::chat::ChatMessage;

pub mod error;
pub mod responses;

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
            match mappings.get(name) {
                Some(mapped) => {
                    existing.insert(mapped.clone(), value.clone());
                }
                None => {
                    if supported.iter().any(|param| param == name) {
                        existing.insert(name.clone(), value.clone());
                    }
                }
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
            value = clamp_below_min_with_n(existing, value, min_with_n)?;
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
        let collapsed_value = serde_json::to_value(collapsed)
            .map_err(|source| JsonProviderPolicyError::Message { source })?;
        serde_json::from_value(collapsed_value)
            .map_err(|source| JsonProviderPolicyError::Message { source })
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

// the min_with_n constraint applies only while n is above one, and a null or non-numeric n is an error
fn clamp_below_min_with_n(
    existing: &Object,
    value: Json,
    min_with_n: Decimal,
) -> Result<Json, JsonProviderPolicyError> {
    let count = match existing.get("n") {
        None => Decimal::ONE,
        Some(Json::Null) => return Err(JsonProviderPolicyError::NNotANumber),
        Some(requested) => {
            decimal_of(requested).map_err(|_error| JsonProviderPolicyError::NNotANumber)?
        }
    };
    if count <= Decimal::ONE {
        return Ok(value);
    }
    let current = decimal_of(&value)?;
    if current < min_with_n {
        return Ok(serde_json::from_str(&min_with_n.to_string()).unwrap_or(Json::Null));
    }
    Ok(value)
}

fn decimal_of(value: &Json) -> Result<Decimal, JsonProviderPolicyError> {
    match value {
        Json::Number(number) => number
            .to_string()
            .parse()
            .map_err(|_error| JsonProviderPolicyError::TemperatureNotANumber),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(JsonProviderPolicyError::TemperatureNotANumber)
        }
    }
}
