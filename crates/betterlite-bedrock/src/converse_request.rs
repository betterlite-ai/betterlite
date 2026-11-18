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
use betterlite_prompts::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use betterlite_prompts::message_content::Message;
use serde::Serialize;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::catalog_flags::{
    accepts_cache_points, is_claude_4_5_on_bedrock,
};
use crate::bedrock_model_capabilities::strict_tools::converse_supports_strict_tools;
use crate::converse_messages::{ConverseMessagePolicy, transform_messages};
use crate::converse_request::body::PrepareRequest;
use crate::converse_request::error::ConverseRequestError;
use crate::converse_request::tools::ToolsRequest;
use crate::event_frames::beta::BetaHeader;
use crate::model_routes::ParamMap;
use crate::request_policy::BedrockRequestPolicy;

const DROP_PARAMS_FIELD: &str = "drop_params";
const USER_CONTINUE_FIELD: &str = "user_continue_message";

// The Converse body fields every request carries, in the order the pinned module inserts them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CommonRequest {
    #[serde(rename = "inferenceConfig")]
    pub inference_config: Object,
    #[serde(
        rename = "additionalModelRequestFields",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_model_request_fields: Option<Object>,
    #[serde(
        rename = "additionalModelResponseFieldPaths",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_model_response_field_paths: Option<Vec<String>>,
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConverseRequestBody {
    pub messages: Vec<Json>,
    #[serde(flatten)]
    pub common: CommonRequest,
}

#[derive(Clone, Copy, Debug)]
pub struct ConverseSettings<'request> {
    pub global_drop: bool,
    pub modify_params: bool,
    pub neutralize_orphans: bool,
    // Some policy owns requestMetadata: the caller's copy is replaced even when nothing resolves.
    pub request_metadata: Option<&'request BedrockRequestPolicy>,
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
) -> Result<ConverseRequestBody, ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let call_drop = input
        .litellm_params
        .as_ref()
        .is_some_and(|params| matches!(params.get(DROP_PARAMS_FIELD), Some(Json::Bool(true))));
    let drop = DropParams {
        global: input.settings.global_drop,
        call: call_drop,
    };
    let transformed = transform_messages(
        input.messages,
        &mut *input.optional_params,
        &ConverseMessagePolicy {
            model: Some(input.model),
            model_cost: input.model_cost,
            neutralize: input.settings.neutralize_orphans,
            modify_params: input.settings.modify_params,
        },
    )?;
    thinking::apply_history_policy(
        &transformed.messages,
        &mut *input.optional_params,
        input.settings.modify_params,
    )?;
    thinking::drop_disabled(
        input.model,
        &mut *input.optional_params,
        environment,
        input.catalogs,
    )
    .await?;
    let mut prepared = body::prepare(
        &PrepareRequest {
            model: input.model,
            drop,
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
    let capabilities = BedrockToolCapabilities {
        cache_points,
        supports_strict_tools: converse_supports_strict_tools(
            input.model,
            input.lookup,
            input.packaged,
        )
        .await,
    };
    let outcome = tools::process(
        &ToolsRequest {
            model: input.model,
            model_cost: input.model_cost,
            headers: input.headers,
            capabilities,
            accepts_cache_points: !matches!(cache_points, CachePointSupport::Unsupported),
        },
        &mut prepared,
        input.litellm_params.as_deref_mut(),
        environment,
        input.catalogs,
    )
    .await?;

    let user_continue_message = input
        .litellm_params
        .as_mut()
        .and_then(|params| params.shift_remove(USER_CONTINUE_FIELD))
        .filter(|value| !matches!(value, Json::Null));
    let mut names = outcome.names;
    let converted = messages::convert(
        &messages::MessagesRequest {
            model: input.model,
            messages: &transformed.messages,
            user_continue_message,
            cache_points,
            modify_params: input.settings.modify_params,
            remote: media.remote,
            new_id: media.new_id,
        },
        &mut names,
    )
    .await?;
    let mut common = assembly::common(prepared, transformed.system, outcome.tool_config)?;
    let litellm_params = input.litellm_params.as_deref();
    metadata::resolve(
        input.settings.request_metadata,
        &metadata::params_of(litellm_params),
        &mut common,
    );
    Ok(ConverseRequestBody {
        messages: converted,
        common,
    })
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
