use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::feature_flags::messages::has_thinking_display_updates;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::api::messages_native_prepare::system_turns::normalize_system_turns;
use betterlite_core::api::messages_native_transform::capabilities::AnthropicCapabilities;
use betterlite_core::api::messages_native_transform::error::TransformError;
use betterlite_core::api::messages_native_transform::request::{
    TransformRequest, transform_native_request,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{CapabilityEnvironment, NamedCapability};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::bedrock_messages_beta::beta::BetaHeadersError;
use crate::bedrock_messages_beta::fields::FieldsError;
use crate::bedrock_messages_beta::provider::CUSTOM_LLM_PROVIDER;
use crate::bedrock_messages_beta::web_search::WebSearchError;
use crate::bedrock_model_capabilities::{CapabilityError, ModelInfoLookup};
use crate::event_frames::beta::BetaHeader;
use crate::message_options::effort::clamp_adaptive_effort;
use crate::message_options::{MessageOptionsError, OptionsSources};
use crate::model_routes::ParamMap;

mod body;

#[cfg(test)]
pub(crate) use body::drop_unsupported_output_config;

// AmazonAnthropicClaudeMessagesConfig.DEFAULT_BEDROCK_ANTHROPIC_API_VERSION
pub const BEDROCK_ANTHROPIC_API_VERSION: &str = "bedrock-2023-05-31";
const ADAPTIVE: &str = "adaptive";
const ANTHROPIC_VERSION: &str = "anthropic_version";
const DISPLAY: &str = "display";
const THINKING: &str = "thinking";
const TYPE: &str = "type";
const UPDATES: &str = "updates";

pub struct MessagesRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<Json>,
    pub optional_params: Object,
    pub headers: &'request BTreeMap<String, BetaHeader>,
    pub model_cost: &'request ParamMap,
    // litellm.drop_params, read by the final output_config branch
    pub drop_params: bool,
}

pub struct TransformSources<'context, 'environment, Source, Transport, Lookup>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    pub environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    pub catalogs: &'context CatalogSources<'environment>,
    pub budgets: &'context Settings,
    pub matrix: &'context BetaMatrix,
    pub lookup: &'context Lookup,
    pub packaged: &'context ModelCatalog,
}

#[derive(Debug, Error)]
pub enum MessagesTransformError {
    #[error(transparent)]
    Transform(#[from] TransformError),
    #[error(transparent)]
    Options(#[from] MessageOptionsError),
    #[error(transparent)]
    WebSearch(#[from] WebSearchError),
    #[error(transparent)]
    Beta(#[from] BetaHeadersError),
    #[error(transparent)]
    Fields(#[from] FieldsError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error("the request does not decode as JSON")]
    Json(#[from] serde_json::Error),
}

impl MessagesTransformError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Transform(error) => Some(error.status_code()),
            Self::WebSearch(error) => Some(error.status_code()),
            Self::Beta(error) => error.status_code(),
            Self::Options(_) | Self::Fields(_) | Self::Capability(_) | Self::Json(_) => None,
        }
    }
}

// AmazonAnthropicClaudeMessagesConfig.transform_anthropic_messages_request through the provider
// overrides: remember the requested display, clamp effort, prepare the native request, restore
// the display on adaptive thinking, normalize system turns, then apply the Bedrock invoke rules.
pub async fn transform_anthropic_messages_request<Source, Transport, Lookup>(
    request: &MessagesRequest<'_>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Lookup>,
) -> Result<Object, MessagesTransformError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let requested_display_updates =
        has_thinking_display_updates(request.optional_params.get(THINKING));
    let mut params = declared_params(&request.optional_params)?;
    clamp_adaptive_effort(
        request.model,
        &mut params,
        &mut OptionsSources {
            environment: &mut *sources.environment,
            catalogs: sources.catalogs,
            budgets: sources.budgets,
        },
        sources.lookup,
        sources.packaged,
    )
    .await?;
    let mid_conversation_system = supports_mid_conversation_system(
        request.model,
        &mut *sources.environment,
        sources.catalogs,
    )
    .await;
    let mut body = native_body(request, params, sources).await?;
    restore_display_updates(&mut body, requested_display_updates);
    normalize_system_turns(&mut body, mid_conversation_system);
    body::finish(request, sources, body).await
}

fn declared_params(optional_params: &Object) -> Result<ParamMap, MessagesTransformError> {
    Ok(serde_json::from_value(serde_json::to_value(
        optional_params,
    )?)?)
}

async fn native_body<Source, Transport, Lookup>(
    request: &MessagesRequest<'_>,
    params: ParamMap,
    sources: &mut TransformSources<'_, '_, Source, Transport, Lookup>,
) -> Result<Object, MessagesTransformError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let optional_params = serde_json::from_value::<Object>(serde_json::to_value(&params)?)?;
    let mut capabilities = AnthropicCapabilities {
        environment: &mut *sources.environment,
        catalogs: sources.catalogs,
        budgets: sources.budgets,
        llm_provider: CUSTOM_LLM_PROVIDER,
    };
    transform_native_request(
        &mut capabilities,
        TransformRequest {
            model: request.model,
            messages: request.messages.clone(),
            optional_params,
            custom_llm_provider: CUSTOM_LLM_PROVIDER,
            strips_billing_metadata: false,
        },
    )
    .await
    .map_err(MessagesTransformError::from)
}

async fn supports_mid_conversation_system<Source, Transport>(
    model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    supports_model_capability(
        model,
        NamedCapability::MidConversationSystem.key(),
        CUSTOM_LLM_PROVIDER,
        environment,
        catalogs,
    )
    .await
}

// The base translation may turn thinking adaptive; only then does the requested display survive.
fn restore_display_updates(body: &mut Object, requested: bool) {
    if !requested {
        return;
    }
    let adaptive = matches!(
        body.get(THINKING),
        Some(Json::Object(fields))
            if matches!(fields.get(TYPE), Some(Json::Text(kind)) if kind == ADAPTIVE)
    );
    if !adaptive {
        return;
    }
    let mut thinking = Object::new();
    thinking.insert(TYPE.to_owned(), Json::Text(ADAPTIVE.to_owned()));
    thinking.insert(DISPLAY.to_owned(), Json::Text(UPDATES.to_owned()));
    body.insert(THINKING.to_owned(), Json::Object(thinking));
}
