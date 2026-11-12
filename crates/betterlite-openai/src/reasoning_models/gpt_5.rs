use betterlite_models::model_capability_predicates::{CapabilityRequest, NamedCapability};
use serde_json::Value;

use super::{
    DropParams, ReasoningCapabilities, ReasoningError, copy_supported, effort_level,
    effort_resolves_to_none, is_model_gpt_5_search_model, is_temperature_one,
};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

const SEARCH_SUPPORTED: [&str; 13] = [
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "stream_options",
    "web_search_options",
    "service_tier",
    "safety_identifier",
    "response_format",
    "user",
    "store",
    "verbosity",
    "max_retries",
    "extra_headers",
];
const ALWAYS_UNSUPPORTED: [&str; 8] = [
    "presence_penalty",
    "frequency_penalty",
    "stop",
    "logit_bias",
    "modalities",
    "prediction",
    "audio",
    "web_search_options",
];
const SAMPLING: [&str; 3] = ["logprobs", "top_logprobs", "top_p"];

pub struct Mapping<'a> {
    pub model: &'a str,
    pub non_default_params: &'a mut Object,
    pub optional_params: &'a mut Object,
    pub drop_params: DropParams,
    pub catalog: &'a CatalogCapabilities,
}

pub async fn supported_params<C>(
    capabilities: &mut C,
    model: &str,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str>
where
    C: ReasoningCapabilities,
{
    if is_model_gpt_5_search_model(model) {
        return SEARCH_SUPPORTED.to_vec();
    }
    let mut params = OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
    .supported_params(model, catalog);
    if !params.contains(&"reasoning_effort") {
        params.push("reasoning_effort");
    }
    params.push("verbosity");
    let request = CapabilityRequest {
        model,
        custom_llm_provider: None,
    };
    let tool_choice = capabilities
        .supports_factory(&request, NamedCapability::ToolChoice.key())
        .await;
    let supports_none = capabilities
        .supports_factory(&request, NamedCapability::NoneReasoningEffort.key())
        .await;
    let mut unsupported = ALWAYS_UNSUPPORTED.to_vec();
    if !supports_none {
        unsupported.extend(SAMPLING);
    }
    params.retain(|param| *param != "tool_choice" || tool_choice);
    params.retain(|param| !unsupported.contains(param));
    params
}

pub async fn map_params<C>(
    capabilities: &mut C,
    mapping: &mut Mapping<'_>,
) -> Result<(), ReasoningError>
where
    C: ReasoningCapabilities,
{
    let model = mapping.model;
    if is_model_gpt_5_search_model(model) {
        if let Some(value) = mapping.non_default_params.remove("max_tokens") {
            mapping
                .optional_params
                .insert(String::from("max_completion_tokens"), value);
        }
        copy_supported(
            mapping.non_default_params,
            mapping.optional_params,
            &SEARCH_SUPPORTED,
        );
        return Ok(());
    }
    let raw = match mapping.non_default_params.get("reasoning_effort") {
        Some(value) if !value.is_null() => Some(value.clone()),
        _ => mapping.optional_params.get("reasoning_effort").cloned(),
    };
    let effective = effort_level(raw.as_ref());
    if let Some(Value::Object(fields)) = &raw {
        if let Some(normalized) = fields.get("effort").and_then(Value::as_str) {
            let normalized = Value::String(normalized.to_owned());
            if mapping.non_default_params.contains_key("reasoning_effort") {
                mapping
                    .non_default_params
                    .insert(String::from("reasoning_effort"), normalized.clone());
            }
            if mapping.optional_params.contains_key("reasoning_effort") {
                mapping
                    .optional_params
                    .insert(String::from("reasoning_effort"), normalized);
            }
        }
    }
    let request = CapabilityRequest {
        model,
        custom_llm_provider: None,
    };
    match effective {
        Some("xhigh") => {
            let supported = capabilities
                .supports_factory(&request, "supports_xhigh_reasoning_effort")
                .await;
            if !supported {
                drop_or_reject_effort(mapping, "xhigh")?;
            }
        }
        Some(level @ ("minimal" | "low")) => {
            let key = format!("supports_{level}_reasoning_effort");
            if capabilities
                .is_explicitly_disabled_factory(&request, &key)
                .await
            {
                drop_or_reject_effort(mapping, level)?;
            }
        }
        _ => {}
    }
    if let Some(value) = mapping.non_default_params.remove("max_tokens") {
        mapping
            .optional_params
            .insert(String::from("max_completion_tokens"), value);
    }
    let supports_none = capabilities
        .supports_factory(&request, NamedCapability::NoneReasoningEffort.key())
        .await;
    let has_sampling = SAMPLING
        .iter()
        .any(|param| mapping.non_default_params.contains_key(*param));
    if supports_none && has_sampling {
        let resolves_to_none = effort_resolves_to_none(capabilities, model, effective).await;
        if !resolves_to_none {
            if mapping.drop_params.global || mapping.drop_params.request {
                for param in SAMPLING {
                    mapping.non_default_params.remove(param);
                }
            } else {
                return Err(ReasoningError::UnsupportedSampling {
                    model: model.to_owned(),
                    effort: effective.map(str::to_owned),
                });
            }
        }
    }
    if let Some(temperature) = mapping.non_default_params.remove("temperature") {
        if !temperature.is_null() {
            let retained = is_temperature_one(&temperature)
                || (supports_none && effort_resolves_to_none(capabilities, model, effective).await);
            if retained {
                mapping
                    .optional_params
                    .insert(String::from("temperature"), temperature);
            } else if !(mapping.drop_params.global || mapping.drop_params.request) {
                return Err(ReasoningError::UnsupportedTemperature {
                    model: model.to_owned(),
                    temperature,
                });
            }
        }
    }
    let supported = supported_params(capabilities, model, mapping.catalog).await;
    copy_supported(
        mapping.non_default_params,
        mapping.optional_params,
        &supported,
    );
    Ok(())
}

fn drop_or_reject_effort(mapping: &mut Mapping<'_>, effort: &str) -> Result<(), ReasoningError> {
    if mapping.drop_params.global || mapping.drop_params.request {
        mapping.non_default_params.remove("reasoning_effort");
        mapping.optional_params.remove("reasoning_effort");
        return Ok(());
    }
    Err(ReasoningError::UnsupportedEffort {
        effort: effort.to_owned(),
    })
}
