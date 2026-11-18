pub mod error;

mod assembly;
mod body;
mod convert;
mod messages;
mod metadata;
mod output_config;
mod records;
mod tests;
mod thinking;
mod tools;

use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_http::user_url::validate::Resolve;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_images::remote::RemoteImage;
use betterlite_prompts::bedrock_tools::capabilities::CachePointSupport;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use betterlite_prompts::message_content::Message;
use serde::Serialize;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::catalog_flags::{
    accepts_cache_points, is_claude_4_5_on_bedrock,
};
use crate::converse_messages::{ConverseMessagePolicy, TransformedMessages, transform_messages};
use crate::converse_request::body::{PrepareRequest, PreparedRequest};
use crate::converse_request::convert::request_params;
use crate::converse_request::error::ConverseRequestError;
use crate::event_frames::beta::BetaHeader;
use crate::model_routes::ParamMap;
use crate::request_policy::{BedrockRequestPolicy, Params};

const DROP_PARAMS_FIELD: &str = "drop_params";

// The Converse body fields every request carries, in the order the pinned module inserts them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CommonRequest {
    #[serde(rename = "inferenceConfig")]
    pub inference_config: Object,
    #[serde(
        rename = "additionalModelRequestFields",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_fields: Option<Object>,
    #[serde(
        rename = "additionalModelResponseFieldPaths",
        skip_serializing_if = "Option::is_none"
    )]
    pub response_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub system: Vec<Json>,
    #[serde(rename = "guardrailConfig", skip_serializing_if = "Option::is_none")]
    pub guardrail_config: Option<Json>,
    #[serde(rename = "performanceConfig", skip_serializing_if = "Option::is_none")]
    pub performance_config: Option<Json>,
    #[serde(rename = "serviceTier", skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<Json>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<Json>,
    #[serde(rename = "requestMetadata", skip_serializing_if = "Option::is_none")]
    pub request_metadata: Option<Json>,
    #[serde(rename = "outputConfig", skip_serializing_if = "Option::is_none")]
    pub output_config: Option<Json>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConverseRequestBody {
    pub messages: Vec<Json>,
    #[serde(flatten)]
    pub common: CommonRequest,
}

// The wire body and the Bedrock-to-caller tool names the reply conversion restores.
pub struct ConverseRequestOutcome {
    pub body: ConverseRequestBody,
    pub tool_name_mappings: ToolNameMap,
}

#[derive(Clone, Copy, Debug)]
pub struct ConverseSettings<'request> {
    pub global_drop: bool,
    pub messages: ConverseMessageSettings,
    // Some policy owns requestMetadata: the caller's copy is replaced even when nothing resolves.
    pub request_metadata: Option<&'request BedrockRequestPolicy>,
}

// The two switches the message policies read; grouping keeps the settings to one bool besides them.
#[derive(Clone, Copy, Debug)]
pub struct ConverseMessageSettings {
    pub modify_params: bool,
    pub neutralize_orphans: bool,
}

pub struct ConverseRequestInput<'request, Lookup> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request mut ParamMap,
    pub litellm_params: Option<&'request mut Object>,
    pub headers: Option<&'request BTreeMap<String, BetaHeader>>,
    pub settings: ConverseSettings<'request>,
    pub model_cost: &'request ParamMap,
    pub catalogs: &'request CatalogSources<'request>,
    pub lookup: &'request Lookup,
    pub packaged: &'request ModelCatalog,
}

pub struct ConverseMedia<'request, R, Ids> {
    pub remote: &'request RemoteImage<R>,
    pub new_id: &'request Ids,
}

// transform_request and _async_transform_request: one body for the sync and async entry points.
pub async fn converse_request<Source, Transport, Lookup, R, Ids>(
    mut input: ConverseRequestInput<'_, Lookup>,
    media: ConverseMedia<'_, R, Ids>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<ConverseRequestOutcome, ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let (transformed, mut prepared, cache_points) =
        prepare_options(&mut input, environment).await?;
    let outcome =
        tools::process_tools(&mut input, &mut prepared, cache_points, environment).await?;
    let mut tool_name_mappings = outcome.names;
    let converted = messages::convert_messages(
        &mut input,
        &transformed,
        &mut tool_name_mappings,
        cache_points,
        &media,
    )
    .await?;
    let mut common = assembly::common(prepared, transformed.system, outcome.tool_config)?;
    metadata::resolve(
        input.settings.request_metadata,
        &input
            .litellm_params
            .as_deref()
            .map_or_else(Params::new, request_params),
        &mut common,
    );
    Ok(ConverseRequestOutcome {
        body: ConverseRequestBody {
            messages: converted,
            common,
        },
        tool_name_mappings,
    })
}

// The message policies run on the caller's options before the option preparation reads them.
async fn prepare_options<Source, Transport, Lookup>(
    input: &mut ConverseRequestInput<'_, Lookup>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(TransformedMessages, PreparedRequest, CachePointSupport), ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let transformed = transform_messages(
        input.messages,
        &mut *input.optional_params,
        &ConverseMessagePolicy {
            model: Some(input.model),
            model_cost: input.model_cost,
            neutralize: input.settings.messages.neutralize_orphans,
            modify_params: input.settings.messages.modify_params,
        },
    )?;
    thinking::apply_history_policy(
        &transformed.messages,
        &mut *input.optional_params,
        input.settings.messages.modify_params,
    )?;
    thinking::drop_disabled(
        input.model,
        &mut *input.optional_params,
        environment,
        input.catalogs,
    )
    .await?;
    let prepared = body::prepare(
        &PrepareRequest {
            model: input.model,
            drop: request_drop(input.litellm_params.as_deref(), input.settings.global_drop),
            model_cost: input.model_cost,
            lookup: input.lookup,
            packaged: input.packaged,
            catalogs: input.catalogs,
        },
        &mut *input.optional_params,
        environment,
    )
    .await?;
    let cache_points = cache_point_support(input.model, input.model_cost)?;
    Ok((transformed, prepared, cache_points))
}

// A call-level drop_params only joins the global setting; the two are never merged into one flag.
fn request_drop(litellm_params: Option<&Object>, global: bool) -> DropParams {
    let requested = litellm_params.and_then(|params| params.get(DROP_PARAMS_FIELD));
    let call = matches!(requested, Some(Json::Bool(true)));
    DropParams { global, call }
}

// An unsupported model places no cache points; Claude 4.5 on Bedrock additionally keeps a ttl.
fn cache_point_support(
    model: &str,
    model_cost: &ParamMap,
) -> Result<CachePointSupport, ConverseRequestError> {
    if !accepts_cache_points(Some(model), model_cost)? {
        return Ok(CachePointSupport::Unsupported);
    }
    if is_claude_4_5_on_bedrock(model, model_cost)? {
        return Ok(CachePointSupport::ExtendedTtl);
    }
    Ok(CachePointSupport::ShortTtl)
}
