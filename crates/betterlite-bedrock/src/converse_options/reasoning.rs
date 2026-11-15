use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::capabilities::effort::EffortError;
use betterlite_anthropic::capabilities::reasoning::{ReasoningRequest, map_reasoning_effort};
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::effort::normalize_opus_effort;
use crate::converse_options::effort::{
    output_config_effort, transform_reasoning_effort_to_reasoning_config,
    validate_anthropic_adaptive_effort,
};
use crate::converse_options::error::ConverseOptionsError;
use crate::converse_options::family::{
    DEEPSEEK_MARKER, is_nova_2_model, is_openai_gpt_reasoning_model,
};
use crate::model_routes::{ParamMap, ParamValue};

const ADAPTIVE_CAPABILITY: &str = "supports_adaptive_thinking";
const CONVERSE_PROVIDER: &str = "bedrock_converse";
const EFFORT_FIELD: &str = "effort";
const NORMALIZED_FIELD: &str = "_output_config_normalized";
const OUTPUT_CONFIG_FIELD: &str = "output_config";
const THINKING_FIELD: &str = "thinking";

// The model-capability probe reads the cost map under bedrock; bedrock_converse only tags raised errors.
pub struct ReasoningSources<'source, Lookup> {
    pub catalogs: &'source CatalogSources<'source>,
    pub budgets: &'source Settings,
    pub lookup: &'source Lookup,
    pub packaged: &'source ModelCatalog,
}

pub async fn handle_reasoning_effort_parameter<Source, Transport, Lookup>(
    model: &str,
    reasoning_effort: &str,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    sources: &ReasoningSources<'_, Lookup>,
) -> Result<(), ConverseOptionsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    if model.contains("gpt-oss") || model.contains(DEEPSEEK_MARKER) {
        optional_params.insert(
            "reasoning_effort".to_owned(),
            ParamValue::Text(reasoning_effort.to_owned()),
        );
        return Ok(());
    }
    if is_openai_gpt_reasoning_model(model) {
        let mut reasoning = ParamMap::new();
        reasoning.insert(
            EFFORT_FIELD.to_owned(),
            ParamValue::Text(reasoning_effort.to_owned()),
        );
        optional_params.insert("reasoning".to_owned(), ParamValue::Mapping(reasoning));
        return Ok(());
    }
    if is_nova_2_model(model) {
        optional_params.extend(transform_reasoning_effort_to_reasoning_config(
            reasoning_effort,
        )?);
        return Ok(());
    }
    map_anthropic_reasoning_effort(
        model,
        reasoning_effort,
        optional_params,
        environment,
        sources,
    )
    .await
}

async fn map_anthropic_reasoning_effort<Source, Transport, Lookup>(
    model: &str,
    reasoning_effort: &str,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    sources: &ReasoningSources<'_, Lookup>,
) -> Result<(), ConverseOptionsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let request = ReasoningRequest {
        reasoning_effort: Some(reasoning_effort),
        model,
        custom_llm_provider: "bedrock",
        llm_provider: CONVERSE_PROVIDER,
        budgets: sources.budgets,
    };
    let Some(thinking) = map_reasoning_effort(&request, environment, sources.catalogs).await?
    else {
        optional_params.shift_remove(THINKING_FIELD);
        optional_params.shift_remove(OUTPUT_CONFIG_FIELD);
        return Ok(());
    };
    optional_params.insert(THINKING_FIELD.to_owned(), param_value(thinking));
    let adaptive = supports_model_capability(
        model,
        ADAPTIVE_CAPABILITY,
        "bedrock",
        environment,
        sources.catalogs,
    )
    .await;
    if !adaptive {
        return Ok(());
    }
    let Some(mapped_effort) = output_config_effort(reasoning_effort) else {
        return Err(EffortError::InvalidReasoningEffort {
            value: reasoning_effort.to_owned(),
            model: model.to_owned(),
            provider: CONVERSE_PROVIDER.to_owned(),
        }
        .into());
    };
    let mut output_config = ParamValue::Mapping(match optional_params.get(OUTPUT_CONFIG_FIELD) {
        Some(ParamValue::Mapping(fields)) => fields.clone(),
        Some(
            ParamValue::Null
            | ParamValue::Bool(_)
            | ParamValue::Number(_)
            | ParamValue::Text(_)
            | ParamValue::List(_),
        )
        | None => ParamMap::new(),
    });
    if let ParamValue::Mapping(fields) = &mut output_config {
        fields
            .entry(EFFORT_FIELD.to_owned())
            .or_insert(ParamValue::Text(mapped_effort.to_owned()));
    }
    normalize_opus_effort(model, &mut output_config, sources.lookup, sources.packaged).await?;
    let effort = match &output_config {
        ParamValue::Mapping(fields) => fields
            .get(EFFORT_FIELD)
            .cloned()
            .unwrap_or(ParamValue::Null),
        ParamValue::Null
        | ParamValue::Bool(_)
        | ParamValue::Number(_)
        | ParamValue::Text(_)
        | ParamValue::List(_) => ParamValue::Null,
    };
    validate_anthropic_adaptive_effort(model, &effort, environment, sources.catalogs).await?;
    optional_params.insert(OUTPUT_CONFIG_FIELD.to_owned(), output_config);
    optional_params.insert(NORMALIZED_FIELD.to_owned(), ParamValue::Bool(true));
    Ok(())
}

fn param_value(value: Json) -> ParamValue {
    match value {
        Json::Null => ParamValue::Null,
        Json::Bool(flag) => ParamValue::Bool(flag),
        Json::Number(number) => ParamValue::Number(number),
        Json::Text(text) => ParamValue::Text(text),
        Json::List(items) => ParamValue::List(items.into_iter().map(param_value).collect()),
        Json::Object(fields) => ParamValue::Mapping(
            fields
                .into_iter()
                .map(|(key, item)| (key, param_value(item)))
                .collect(),
        ),
    }
}
