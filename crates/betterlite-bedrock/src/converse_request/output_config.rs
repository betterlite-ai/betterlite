use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::capabilities::effort_param::model_supports_effort_param;
use betterlite_anthropic::params::output_config::DROP_OUTPUT_CONFIG_WARNING;
use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::effort::normalize_opus_effort;
use crate::converse_options::config::CUSTOM_LLM_PROVIDER;
use crate::converse_options::effort::validate_adaptive_effort;
use crate::converse_params::wire::transcode;
use crate::converse_request::error::ConverseRequestError;
use crate::converse_tools::response_format::create_output_config;
use crate::model_routes::ParamValue;

const ANTHROPIC_PREFIX: &str = "anthropic";
const APPLICATION_PROFILE_MARKER: &str = ":application-inference-profile/";
const BEDROCK_PROVIDER: &str = "bedrock";
const DESCRIPTION_FIELD: &str = "description";
const EFFORT_FIELD: &str = "effort";
const JSON_SCHEMA: &str = "json_schema";
const NAME_FIELD: &str = "name";
const NATIVE_STRUCTURED_OUTPUT: &str = "supports_native_structured_output";
const SCHEMA_FIELD: &str = "schema";
const TYPE_FIELD: &str = "type";

pub(super) struct OutputRequest<'request, Lookup> {
    pub(super) model: &'request str,
    pub(super) already_normalized: bool,
    pub(super) drop: DropParams,
    pub(super) lookup: &'request Lookup,
    pub(super) packaged: &'request ModelCatalog,
    pub(super) catalogs: &'request CatalogSources<'request>,
}

pub(super) struct OutputCandidates {
    pub(super) anthropic: Option<ParamValue>,
    pub(super) format: Option<ParamValue>,
    pub(super) native: Option<ParamValue>,
}

pub(super) struct OutputDecision {
    pub(super) output_config: Option<Json>,
    pub(super) additional: Option<ParamValue>,
}

pub(super) async fn decide<Source, Transport, Lookup>(
    request: &OutputRequest<'_, Lookup>,
    candidates: OutputCandidates,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<OutputDecision, ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let base_model = get_bedrock_base_model(request.model);
    let output_config = native_config(
        request,
        candidates.native,
        candidates.format.as_ref(),
        &base_model,
        environment,
    )
    .await?;
    let additional = match candidates.anthropic {
        Some(config @ ParamValue::Mapping(_)) => {
            apply_anthropic(request, config, &base_model, environment).await?
        }
        Some(
            ParamValue::Null
            | ParamValue::Bool(_)
            | ParamValue::Number(_)
            | ParamValue::Text(_)
            | ParamValue::List(_),
        )
        | None => None,
    };
    Ok(OutputDecision {
        output_config,
        additional,
    })
}

async fn native_config<Source, Transport, Lookup>(
    request: &OutputRequest<'_, Lookup>,
    native: Option<ParamValue>,
    format: Option<&ParamValue>,
    base_model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Option<Json>, ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    if let Some(value) = native {
        return Ok(Some(transcode(&value)?));
    }
    let Some(ParamValue::Mapping(fields)) = format else {
        return Ok(None);
    };
    let is_schema = matches!(
        fields.get(TYPE_FIELD),
        Some(ParamValue::Text(kind)) if kind.as_str() == JSON_SCHEMA
    );
    let supported = is_schema
        && base_model.starts_with(ANTHROPIC_PREFIX)
        && supports_model_capability(
            request.model,
            NATIVE_STRUCTURED_OUTPUT,
            CUSTOM_LLM_PROVIDER,
            environment,
            request.catalogs,
        )
        .await;
    if !supported {
        tracing::warn!(
            "Bedrock Converse: dropping `output_config.format` for model={} — model does not advertise `supports_native_structured_output` in model_prices_and_context_window.json. The schema will not be enforced; pass `response_format` to use the synthetic tool-call fallback.",
            request.model
        );
        return Ok(None);
    }
    let schema: Option<DecisionsJson> = match fields.get(SCHEMA_FIELD) {
        Some(value) => Some(transcode(value)?),
        None => None,
    };
    let name = text_field(fields, NAME_FIELD);
    let description = text_field(fields, DESCRIPTION_FIELD);
    let created = create_output_config(schema.as_ref(), name, description);
    Ok(Some(transcode(&created)?))
}

// Only a text member names the output; any other value is the same as an absent one.
fn text_field<'fields>(
    fields: &'fields IndexMap<String, ParamValue>,
    key: &str,
) -> Option<&'fields str> {
    match fields.get(key) {
        Some(ParamValue::Text(text)) => Some(text.as_str()),
        Some(
            ParamValue::Null
            | ParamValue::Bool(_)
            | ParamValue::Number(_)
            | ParamValue::List(_)
            | ParamValue::Mapping(_),
        )
        | None => None,
    }
}

async fn apply_anthropic<Source, Transport, Lookup>(
    request: &OutputRequest<'_, Lookup>,
    mut config: ParamValue,
    base_model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Option<ParamValue>, ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    // The ARN hides the model, so the effort gates cannot run and Bedrock enforces the value
    if request.model.contains(APPLICATION_PROFILE_MARKER) {
        return Ok(Some(config));
    }
    if !base_model.starts_with(ANTHROPIC_PREFIX) {
        return Ok(None);
    }
    let unsupported = request.drop.global
        && !model_supports_effort_param(
            request.model,
            BEDROCK_PROVIDER,
            environment,
            request.catalogs,
        )
        .await;
    if unsupported {
        tracing::warn!(
            "{}",
            DROP_OUTPUT_CONFIG_WARNING.replacen("%s", request.model, 1)
        );
        return Ok(None);
    }
    if !request.already_normalized {
        normalize_opus_effort(request.model, &mut config, request.lookup, request.packaged).await?;
    }
    if let ParamValue::Mapping(fields) = &config
        && let Some(effort) = fields.get(EFFORT_FIELD)
    {
        validate_adaptive_effort(request.model, effort, environment, request.catalogs).await?;
    }
    Ok(Some(config))
}
