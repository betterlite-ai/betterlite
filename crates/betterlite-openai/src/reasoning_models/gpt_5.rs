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

pub struct Mapping<'request> {
    pub model: &'request str,
    pub non_default_params: &'request mut Object,
    pub optional_params: &'request mut Object,
    pub drop_params: DropParams,
    pub catalog: &'request CatalogCapabilities,
}

struct Resolution<'request> {
    model: &'request str,
    effort: Option<&'request str>,
    supports_none: bool,
    resolves_none: bool,
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
        map_max_tokens(mapping);
        copy_supported(
            mapping.non_default_params,
            mapping.optional_params,
            &SEARCH_SUPPORTED,
        );
        return Ok(());
    }
    map_regular_params(capabilities, mapping, model).await
}

async fn map_regular_params<C>(
    capabilities: &mut C,
    mapping: &mut Mapping<'_>,
    model: &str,
) -> Result<(), ReasoningError>
where
    C: ReasoningCapabilities,
{
    let raw = raw_reasoning_effort(mapping);
    let effective = effort_level(raw.as_ref()).map(str::to_owned);
    normalize_effort(mapping, raw.as_ref());
    let request = CapabilityRequest {
        model,
        custom_llm_provider: None,
    };
    apply_effort_guard(capabilities, &request, effective.as_deref(), mapping).await?;
    map_max_tokens(mapping);
    let supports_none = capabilities
        .supports_factory(&request, NamedCapability::NoneReasoningEffort.key())
        .await;
    let resolution = Resolution {
        model,
        effort: effective.as_deref(),
        supports_none,
        resolves_none: effort_resolves_to_none(capabilities, model, effective.as_deref()).await,
    };
    apply_sampling_guard(mapping, &resolution)?;
    apply_temperature_guard(mapping, &resolution)?;
    let supported = supported_params(capabilities, model, mapping.catalog).await;
    copy_supported(
        mapping.non_default_params,
        mapping.optional_params,
        &supported,
    );
    Ok(())
}

fn raw_reasoning_effort(mapping: &Mapping<'_>) -> Option<Value> {
    match mapping.non_default_params.get("reasoning_effort") {
        Some(value) if !value.is_null() => Some(value.clone()),
        Some(_) | None => mapping.optional_params.get("reasoning_effort").cloned(),
    }
}

fn normalize_effort(mapping: &mut Mapping<'_>, raw: Option<&Value>) {
    let Some(Value::Object(fields)) = raw else {
        return;
    };
    let Some(normalized) = fields.get("effort").and_then(Value::as_str) else {
        return;
    };
    let normalized_value = Value::String(normalized.to_owned());
    if mapping.non_default_params.contains_key("reasoning_effort") {
        mapping
            .non_default_params
            .insert(String::from("reasoning_effort"), normalized_value.clone());
    }
    if mapping.optional_params.contains_key("reasoning_effort") {
        mapping
            .optional_params
            .insert(String::from("reasoning_effort"), normalized_value);
    }
}

async fn apply_effort_guard<C>(
    capabilities: &mut C,
    request: &CapabilityRequest<'_>,
    effort: Option<&str>,
    mapping: &mut Mapping<'_>,
) -> Result<(), ReasoningError>
where
    C: ReasoningCapabilities,
{
    let rejected = match effort {
        Some("xhigh")
            if !capabilities
                .supports_factory(request, "supports_xhigh_reasoning_effort")
                .await =>
        {
            "xhigh"
        }
        Some(level @ ("minimal" | "low"))
            if capabilities
                .is_explicitly_disabled_factory(
                    request,
                    &format!("supports_{level}_reasoning_effort"),
                )
                .await =>
        {
            level
        }
        Some(_) | None => return Ok(()),
    };
    reject_unless_dropping(
        mapping,
        ReasoningError::UnsupportedEffort {
            effort: rejected.to_owned(),
        },
    )?;
    mapping.non_default_params.remove("reasoning_effort");
    mapping.optional_params.remove("reasoning_effort");
    Ok(())
}

fn map_max_tokens(mapping: &mut Mapping<'_>) {
    if let Some(value) = mapping.non_default_params.remove("max_tokens") {
        mapping
            .optional_params
            .insert(String::from("max_completion_tokens"), value);
    }
}

fn apply_sampling_guard(
    mapping: &mut Mapping<'_>,
    resolution: &Resolution<'_>,
) -> Result<(), ReasoningError> {
    let present: Vec<&str> = SAMPLING
        .into_iter()
        .filter(|param| mapping.non_default_params.contains_key(*param))
        .collect();
    if present.is_empty() || !resolution.supports_none || resolution.resolves_none {
        return Ok(());
    }
    reject_unless_dropping(
        mapping,
        ReasoningError::UnsupportedSampling {
            model: resolution.model.to_owned(),
            effort: resolution.effort.map(str::to_owned),
        },
    )?;
    for param in present {
        mapping.non_default_params.remove(param);
    }
    Ok(())
}

fn apply_temperature_guard(
    mapping: &mut Mapping<'_>,
    resolution: &Resolution<'_>,
) -> Result<(), ReasoningError> {
    let Some(temperature) = mapping.non_default_params.remove("temperature") else {
        return Ok(());
    };
    let allowed =
        is_temperature_one(&temperature) || (resolution.supports_none && resolution.resolves_none);
    match (temperature, allowed) {
        (Value::Null, _) => Ok(()),
        (kept, true) => {
            mapping
                .optional_params
                .insert(String::from("temperature"), kept);
            Ok(())
        }
        (rejected, false) => reject_unless_dropping(
            mapping,
            ReasoningError::UnsupportedTemperature {
                model: resolution.model.to_owned(),
                temperature: rejected,
            },
        ),
    }
}

fn reject_unless_dropping(
    mapping: &Mapping<'_>,
    error: ReasoningError,
) -> Result<(), ReasoningError> {
    if mapping.drop_params.global || mapping.drop_params.request {
        Ok(())
    } else {
        Err(error)
    }
}
