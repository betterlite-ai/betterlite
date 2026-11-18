use std::collections::BTreeSet;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_context::request_records::RequestRecordProjection;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::catalog_flags::supports_parallel_tools;
use crate::converse_params::inference::handle_top_k_value;
use crate::converse_params::merge::merge_parallel_tool_use_config;
use crate::converse_params::wire::{carries_value, transcode};
use crate::converse_request::convert::request_value;
use crate::converse_request::error::ConverseRequestError;
use crate::converse_request::output_config::{self, OutputCandidates, OutputRequest};
use crate::converse_request::records::{MAX_DEPTH, params_of, record_of, safe_copy};
use crate::model_routes::{ParamMap, ParamValue};
use crate::request_policy::validation::validate_request_metadata;

const CLIENT_METADATA_FIELD: &str = "client_metadata";
const FORMAT_FIELD: &str = "format";
const JSON_MODE_FIELD: &str = "json_mode";
const OUTPUT_CONFIG_CAMEL_FIELD: &str = "outputConfig";
const OUTPUT_CONFIG_FIELD: &str = "output_config";
const OUTPUT_CONFIG_MARKER: &str = "_output_config_normalized";
const PARALLEL_TOOL_CALLS_FIELD: &str = "parallel_tool_calls";
const PARALLEL_TOOL_USE_FIELD: &str = "_parallel_tool_use_config";
const REQUEST_METADATA_FIELD: &str = "requestMetadata";
const INFERENCE_FIELDS: [&str; 6] = [
    "maxTokens",
    "stopSequences",
    "temperature",
    "topP",
    "topK",
    "top_k",
];
const TOOL_FIELDS: [&str; 2] = ["tools", "tool_choice"];
const CONFIG_FIELDS: [&str; 3] = ["guardrailConfig", "performanceConfig", "serviceTier"];

pub(super) struct PrepareRequest<'request, Lookup> {
    pub(super) model: &'request str,
    pub(super) drop: DropParams,
    pub(super) model_cost: &'request ParamMap,
    pub(super) lookup: &'request Lookup,
    pub(super) packaged: &'request ModelCatalog,
    pub(super) catalogs: &'request CatalogSources<'request>,
}

pub(super) struct PreparedRequest {
    pub(super) inference_params: ParamMap,
    pub(super) additional: ParamMap,
    pub(super) request_metadata: Option<Json>,
    pub(super) output_config: Option<Json>,
}

// _prepare_request_params: the caller's options are only read here except for the internal marker.
pub(super) async fn prepare<Source, Transport, Lookup>(
    request: &PrepareRequest<'_, Lookup>,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<PreparedRequest, ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let (mut inference_params, already_normalized) = filter_options(optional_params)?;
    let (anthropic_output, format_candidate) =
        split_output_config(inference_params.shift_remove(OUTPUT_CONFIG_FIELD));
    let request_metadata = metadata_value(inference_params.shift_remove(REQUEST_METADATA_FIELD))?;
    let native = inference_params
        .shift_remove(OUTPUT_CONFIG_CAMEL_FIELD)
        .filter(|value| !matches!(value, ParamValue::Null));
    let mut additional = request_additional(request, &mut inference_params)?;

    let decision = output_config::decide(
        &OutputRequest {
            model: request.model,
            already_normalized,
            drop: request.drop,
            lookup: request.lookup,
            packaged: request.packaged,
            catalogs: request.catalogs,
        },
        OutputCandidates {
            anthropic: anthropic_output,
            format: format_candidate,
            native,
        },
        environment,
    )
    .await?;
    if let Some(config) = decision.additional {
        additional.insert(OUTPUT_CONFIG_FIELD.to_owned(), config);
    }
    Ok(PreparedRequest {
        inference_params,
        additional,
        request_metadata,
        output_config: decision.output_config,
    })
}

// The caller's own options lose only the internal marker; the returned copy takes every other key.
fn filter_options(
    optional_params: &mut ParamMap,
) -> Result<(ParamMap, bool), ConverseRequestError> {
    let already_normalized = optional_params
        .shift_remove(OUTPUT_CONFIG_MARKER)
        .is_some_and(|value| carries_value(&value));
    let exceptions =
        RequestRecordProjection::filter_parameters(record_of(optional_params), MAX_DEPTH);
    let mut params = safe_copy(&params_of(&exceptions)?)?;
    params.shift_remove(JSON_MODE_FIELD);
    Ok((params, already_normalized))
}

fn request_additional<Lookup>(
    request: &PrepareRequest<'_, Lookup>,
    inference_params: &mut ParamMap,
) -> Result<ParamMap, ConverseRequestError>
where
    Lookup: ModelInfoLookup,
{
    let mut additional = split_additional(inference_params);
    merge_parallel_config(request, &mut additional)?;
    additional.shift_remove(PARALLEL_TOOL_CALLS_FIELD);
    additional.shift_remove(CLIENT_METADATA_FIELD);
    additional.extend(handle_top_k_value(
        request.model,
        inference_params,
        request.drop,
        request.catalogs,
    )?);
    let internal =
        RequestRecordProjection::filter_internal(record_of(&additional), &BTreeSet::new());
    additional = params_of(&RequestRecordProjection::filter_parameters(
        internal, MAX_DEPTH,
    ))?;
    Ok(additional)
}

// The supported fields keep the caller's order; everything else is a request field.
fn split_additional(inference_params: &mut ParamMap) -> ParamMap {
    let unsupported: Vec<String> = inference_params
        .keys()
        .filter(|key| {
            !(INFERENCE_FIELDS.contains(&key.as_str())
                || TOOL_FIELDS.contains(&key.as_str())
                || CONFIG_FIELDS.contains(&key.as_str()))
        })
        .cloned()
        .collect();
    unsupported
        .into_iter()
        .filter_map(|key| {
            inference_params
                .shift_remove(&key)
                .map(|value| (key, value))
        })
        .collect()
}

// A mapping with no members left is absent, and a format that is not an object is discarded.
fn split_output_config(value: Option<ParamValue>) -> (Option<ParamValue>, Option<ParamValue>) {
    let Some(ParamValue::Mapping(mut fields)) = value else {
        return (None, None);
    };
    let candidate = fields
        .shift_remove(FORMAT_FIELD)
        .filter(|value| matches!(value, ParamValue::Mapping(_)));
    let remaining = (!fields.is_empty()).then_some(ParamValue::Mapping(fields));
    (remaining, candidate)
}

fn metadata_value(value: Option<ParamValue>) -> Result<Option<Json>, ConverseRequestError> {
    match value {
        None | Some(ParamValue::Null) => Ok(None),
        Some(value) => {
            let encoded: Json = transcode(&value)?;
            validate_request_metadata(&request_value(&encoded))?;
            Ok(Some(encoded))
        }
    }
}

// The internal block is merged only on models that support it; otherwise both keys are dropped.
fn merge_parallel_config<Lookup>(
    request: &PrepareRequest<'_, Lookup>,
    additional: &mut ParamMap,
) -> Result<(), ConverseRequestError>
where
    Lookup: ModelInfoLookup,
{
    match additional.shift_remove(PARALLEL_TOOL_USE_FIELD) {
        None | Some(ParamValue::Null) => Ok(()),
        Some(ParamValue::Mapping(config)) => {
            if supports_parallel_tools(request.model, request.model_cost)? {
                *additional = merge_parallel_tool_use_config(additional, &config);
            }
            Ok(())
        }
        Some(
            ParamValue::Bool(_) | ParamValue::Number(_) | ParamValue::Text(_) | ParamValue::List(_),
        ) => Err(ConverseRequestError::FieldType {
            field: PARALLEL_TOOL_USE_FIELD,
            expected: "an object",
        }),
    }
}
