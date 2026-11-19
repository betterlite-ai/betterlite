pub mod error;

mod tests;

use std::error::Error;
use std::future::Future;

use betterlite_models::model_capability_predicates::NamedCapability;
use betterlite_prompts::arguments::json::{Json, Object};

use self::error::NativeVertexMessagesError;
use crate::api::messages_native_prepare::billing::remove_cache_scope;
use crate::api::messages_native_prepare::system_turns::normalize_system_turns;
use crate::api::messages_native_transform::capabilities::ThinkingCapabilities;
use crate::api::messages_native_transform::request::{TransformRequest, transform_native_request};

// LiteLLM's Vertex native Messages config reports this provider whatever the caller passed.
pub const PROVIDER: &str = "vertex_ai";

pub struct NativeMessages<'request> {
    pub model: &'request str,
    pub messages: Vec<Json>,
    pub optional_params: Object,
}

// The Vertex body policy the leaf applies once the parent stages have run; the leaf never sees core.
pub trait VertexMessagesOverrides {
    type Error: Error + Send + Sync + 'static;

    fn apply<'body>(
        &'body mut self,
        body: &'body mut Object,
        model: &'body str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'body;
}

// LiteLLM's transform_anthropic_messages_request: the parent body, then the system and cache normalization, then the Vertex overrides.
pub async fn adapt<Capabilities, Overrides>(
    capabilities: &mut Capabilities,
    request: NativeMessages<'_>,
    overrides: &mut Overrides,
) -> Result<Object, NativeVertexMessagesError<Overrides::Error>>
where
    Capabilities: ThinkingCapabilities + Send,
    Overrides: VertexMessagesOverrides + Send,
{
    let NativeMessages {
        model,
        messages,
        optional_params,
    } = request;
    let mut body = transform_native_request(
        capabilities,
        TransformRequest {
            model,
            messages,
            optional_params,
            custom_llm_provider: PROVIDER,
            strips_billing_metadata: true,
        },
    )
    .await?;
    let supports_mid_system = capabilities
        .supports(model, NamedCapability::MidConversationSystem.key(), PROVIDER)
        .await;
    normalize_system_turns(&mut body, supports_mid_system);
    remove_cache_scope(&mut body);
    overrides
        .apply(&mut body, model)
        .await
        .map_err(NativeVertexMessagesError::Overrides)?;
    Ok(body)
}
