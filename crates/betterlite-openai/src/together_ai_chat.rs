use betterlite_error::error::{ErrorClass, ErrorContext};
use thiserror::Error;

use crate::chat::{ChatMessage, Role};
use crate::openai_base_config::{CatalogCapabilities, Object, OpenAiConfig};
use crate::reasoning_models::DropParams;

pub mod reasoning;
mod tests;

use reasoning::rewrite_reasoning_effort;

const TOOL_CALLING_PARAMS: [&str; 3] = ["tools", "tool_choice", "function_call"];
const INTERNAL_ASSISTANT_FIELDS: [&str; 2] = ["thinking_blocks", "provider_specific_fields"];
const REASONING_EFFORT: &str = "reasoning_effort";
const RESPONSE_FORMAT: &str = "response_format";
const PROVIDER: &str = "together_ai";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TogetherFlag {
    SupportsFunctionCalling,
    SupportsResponseSchema,
    SupportsReasoning,
}

impl TogetherFlag {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SupportsFunctionCalling => "supports_function_calling",
            Self::SupportsResponseSchema => "supports_response_schema",
            Self::SupportsReasoning => "supports_reasoning",
        }
    }
}

// B2221 answers through this trait; a failed lookup is not the same answer as a false one
#[derive(Debug, Error)]
#[error("{message}")]
pub struct LookupFailed {
    pub message: String,
}

// B2221 supplies the capability answers and the stored catalog flag; this module never reads the catalog
pub trait TogetherCapabilities {
    fn check(&self, flag: TogetherFlag, model: &str) -> Result<bool, LookupFailed>;

    fn registry_flag(&self, flag: TogetherFlag, model: &str) -> Option<bool>;

    fn declared_reasoning_efforts(&self, model: &str) -> Option<Vec<String>>;
}

#[derive(Debug, Error)]
pub enum TogetherChatError {
    #[error(
        "together_ai does not support parameters: {params}, for model={model}. To drop it from the call, set `litellm.drop_params = True`."
    )]
    UnsupportedParams { params: String, model: String },
}

impl TogetherChatError {
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::UnsupportedParams { model, .. } => {
                ErrorClass::UnsupportedParams(Box::new(ErrorContext {
                    message: self.to_string(),
                    model: Some(model.clone()),
                    provider: Some(PROVIDER.to_owned()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                }))
            }
        }
    }

    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnsupportedParams { .. } => 400,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TogetherAiChatConfig {
    pub base: OpenAiConfig,
}

pub struct OptionMapping<'request> {
    pub model: &'request str,
    pub non_default_params: &'request Object,
    pub optional_params: &'request mut Object,
    pub catalog: &'request CatalogCapabilities,
    pub drop_params: DropParams,
}

impl TogetherAiChatConfig {
    // LiteLLM strips these before its own message transform runs and keeps reasoning_content for replay
    pub fn strip_internal_fields(messages: &mut [ChatMessage]) {
        for message in messages {
            if message.role != Role::Assistant {
                continue;
            }
            for field in INTERNAL_ASSISTANT_FIELDS {
                message.passthrough.shift_remove(field);
            }
        }
    }

    #[must_use]
    pub fn supported_params(
        &self,
        model: &str,
        catalog: &CatalogCapabilities,
        capabilities: &dyn TogetherCapabilities,
    ) -> Vec<&'static str> {
        let mut params = self.base.supported_params(model, catalog);
        if reasoning::supports_together_reasoning(model, capabilities) {
            params.push(REASONING_EFFORT);
        }
        params
    }

    pub fn map_openai_params(
        &self,
        mapping: OptionMapping<'_>,
        capabilities: &dyn TogetherCapabilities,
    ) -> Result<(), TogetherChatError> {
        let OptionMapping {
            model,
            non_default_params,
            optional_params,
            catalog,
            drop_params,
        } = mapping;
        let supported = self.supported_params(model, catalog, capabilities);
        for (name, value) in non_default_params {
            if supported.contains(&name.as_str()) {
                optional_params.insert(name.clone(), value.clone());
            }
        }
        let passed_tool_params: Vec<&'static str> = TOOL_CALLING_PARAMS
            .into_iter()
            .filter(|param| optional_params.contains_key(*param))
            .collect();
        if !passed_tool_params.is_empty() {
            drop_tool_params(
                &passed_tool_params,
                model,
                drop_params,
                capabilities,
                optional_params,
            )?;
        }
        if optional_params.contains_key(RESPONSE_FORMAT)
            && drop_response_format(model, drop_params, capabilities)?
        {
            optional_params.remove(RESPONSE_FORMAT);
        }
        rewrite_reasoning_effort(optional_params, model, capabilities);
        Ok(())
    }
}

fn verdict(
    flag: TogetherFlag,
    model: &str,
    capabilities: &dyn TogetherCapabilities,
) -> Option<bool> {
    match capabilities.check(flag, model) {
        Ok(true) => return Some(true),
        Ok(false) => {}
        Err(error) => tracing::debug!(
            flag = flag.name(),
            model,
            error = %error,
            "Error checking together_ai capability"
        ),
    }
    (capabilities.registry_flag(flag, model) == Some(false)).then_some(false)
}

fn drop_tool_params(
    passed: &[&'static str],
    model: &str,
    drop_params: DropParams,
    capabilities: &dyn TogetherCapabilities,
    optional_params: &mut Object,
) -> Result<(), TogetherChatError> {
    let listed = passed.join(", ");
    match verdict(TogetherFlag::SupportsFunctionCalling, model, capabilities) {
        Some(true) => Ok(()),
        None => {
            tracing::warn!(
                model,
                params = %listed,
                "together_ai model has no function calling entry in the model registry; passing the parameters through for Together to validate"
            );
            Ok(())
        }
        Some(false) if drop_params.request || drop_params.global => {
            tracing::warn!(
                model,
                params = %listed,
                "together_ai model does not support function calling per the model registry; dropping the parameters"
            );
            for param in passed {
                optional_params.remove(*param);
            }
            Ok(())
        }
        Some(false) => Err(TogetherChatError::UnsupportedParams {
            params: listed,
            model: model.to_owned(),
        }),
    }
}

fn drop_response_format(
    model: &str,
    drop_params: DropParams,
    capabilities: &dyn TogetherCapabilities,
) -> Result<bool, TogetherChatError> {
    match verdict(TogetherFlag::SupportsResponseSchema, model, capabilities) {
        Some(true) => Ok(false),
        None => {
            tracing::warn!(
                model,
                "together_ai model has no structured outputs entry in the model registry; passing response_format through for Together to validate"
            );
            Ok(false)
        }
        Some(false) if drop_params.request || drop_params.global => {
            tracing::warn!(
                model,
                "together_ai model does not support structured outputs per the model registry; dropping response_format"
            );
            Ok(true)
        }
        Some(false) => Err(TogetherChatError::UnsupportedParams {
            params: String::from(RESPONSE_FORMAT),
            model: model.to_owned(),
        }),
    }
}
