use betterlite_models::model_capability_predicates::{CapabilityRequest, NamedCapability};
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object as ParamsObject};
use betterlite_openai::reasoning_models::gpt_5::{self, Mapping as Gpt5Mapping};
use betterlite_openai::reasoning_models::{
    ReasoningCapabilities, effort_level, is_model_gpt_5_2_model,
};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;

use super::capabilities::{AzureCapabilities, GPT5_SERIES_ROUTE};
use super::error::ChatParamsError;
use crate::azure_chat_base_policy::messages::transform_request as base_transform;

pub const LOGPROB_PARAMS: [&str; 2] = ["logprobs", "top_logprobs"];
const TOOL_CHOICE_PARAM: &str = "tool_choice";
const REASONING_EFFORT: &str = "reasoning_effort";
const NONE: &str = "none";

pub async fn supported_params<Capabilities>(
    capabilities: &mut Capabilities,
    model: &str,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str>
where
    Capabilities: ReasoningCapabilities + Send,
{
    let mut azure = AzureCapabilities {
        inner: capabilities,
    };
    let mut params = gpt_5::supported_params(&mut azure, model, catalog).await;
    if !params.contains(&TOOL_CHOICE_PARAM) {
        params.push(TOOL_CHOICE_PARAM);
    }
    let supports_none = supports_none_effort(&mut azure, model).await;
    let is_gpt_5_2 = is_model_gpt_5_2_model(model);
    if supports_none && !is_gpt_5_2 {
        params.retain(|param| !LOGPROB_PARAMS.contains(param));
    }
    if is_gpt_5_2 {
        params.extend(LOGPROB_PARAMS);
    }
    params
}

pub async fn map_params<Capabilities>(
    capabilities: &mut Capabilities,
    mapping: &mut Gpt5Mapping<'_>,
) -> Result<(), ChatParamsError>
where
    Capabilities: ReasoningCapabilities + Send,
{
    let mut azure = AzureCapabilities {
        inner: capabilities,
    };
    let supports_none = supports_none_effort(&mut azure, mapping.model).await;
    let effective = effective_effort(mapping);
    if effective.as_deref() == Some(NONE) && !supports_none {
        if mapping.drop_params.global || mapping.drop_params.request {
            drop_none_effort(mapping);
        } else {
            return Err(ChatParamsError::UnsupportedNone);
        }
    }
    let before = mapping.optional_params.clone();
    gpt_5::map_params(&mut azure, mapping).await?;
    let azure_supported = supported_params(&mut azure, mapping.model, mapping.catalog).await;
    let openai_supported =
        gpt_5::supported_params(&mut azure, mapping.model, mapping.catalog).await;
    copy_azure_allowlist(mapping, &before, &azure_supported, &openai_supported);
    let mapped = mapping.optional_params.get(REASONING_EFFORT);
    if !supports_none && effort_level(mapped) == Some(NONE) {
        mapping.optional_params.remove(REASONING_EFFORT);
    }
    Ok(())
}

// LiteLLM's final copy reads the Azure allowlist, so names only OpenAI's list carries are undone
fn copy_azure_allowlist(
    mapping: &mut Gpt5Mapping<'_>,
    before: &ParamsObject,
    azure_supported: &[&str],
    openai_supported: &[&str],
) {
    for (name, value) in mapping.non_default_params.iter() {
        let in_azure = azure_supported.contains(&name.as_str());
        let in_openai = openai_supported.contains(&name.as_str());
        if in_azure {
            mapping.optional_params.insert(name.clone(), value.clone());
        }
        if !in_azure && in_openai {
            match before.get(name) {
                Some(original) => {
                    mapping
                        .optional_params
                        .insert(name.clone(), original.clone());
                }
                None => {
                    mapping.optional_params.remove(name);
                }
            }
        }
    }
}

pub fn transform_request(
    model: &str,
    messages: &[Message],
    optional: &Object,
    openai_system_messages_first: bool,
) -> Result<Object, ChatParamsError> {
    base_transform(
        &model.replace(GPT5_SERIES_ROUTE, ""),
        messages,
        optional,
        openai_system_messages_first,
    )
    .map_err(ChatParamsError::Messages)
}

async fn supports_none_effort<Capabilities>(
    capabilities: &mut AzureCapabilities<'_, Capabilities>,
    model: &str,
) -> bool
where
    Capabilities: ReasoningCapabilities + Send,
{
    let request = CapabilityRequest {
        model,
        custom_llm_provider: None,
    };
    capabilities
        .supports_factory(&request, NamedCapability::NoneReasoningEffort.key())
        .await
}

fn effective_effort(mapping: &Gpt5Mapping<'_>) -> Option<String> {
    let requested = mapping
        .non_default_params
        .get(REASONING_EFFORT)
        .filter(|value| !value.is_null());
    let present = requested.or_else(|| mapping.optional_params.get(REASONING_EFFORT));
    effort_level(present).map(str::to_owned)
}

fn drop_none_effort(mapping: &mut Gpt5Mapping<'_>) {
    let requested = mapping.non_default_params.get(REASONING_EFFORT);
    if effort_level(requested) == Some(NONE) {
        mapping.non_default_params.remove(REASONING_EFFORT);
    }
    let existing = mapping.optional_params.get(REASONING_EFFORT);
    if effort_level(existing) == Some(NONE) {
        mapping.optional_params.remove(REASONING_EFFORT);
    }
}
