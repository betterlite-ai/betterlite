#![expect(
    clippy::pub_use,
    reason = "the task re-exports B2413's primitives through this stream surface"
)]

use std::error::Error;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_stream_hooks::ResponsesStreamHooks;
use crate::api::responses_stream_hooks::ports::{Dispatch, PortFailure};

pub use crate::api::responses_stream_primitives::decode::{
    JsonShape, StreamDecodeError, decode_responses_stream_value,
};
pub use crate::api::responses_stream_primitives::state::{
    BaseStreamStateInput, LoggingSnapshot, ResponsesApiBase, ResponsesStreamBaseState,
    ResponsesStreamHiddenParams, SharedResponsesApiBase, StreamTimes, base_stream_state,
};

pub mod affinity;
pub mod deltas;
pub mod identity;
pub mod record;
pub mod terminal;

mod pipeline;
mod tests;

// B943-B946 enforce this per frame; processing an ordinary SSE event never calls it
#[async_trait]
pub trait ProjectQuotaCallback: Send + Sync {
    async fn enforce_frame_quota(
        &self,
        user_api_key_dict: Option<&UserAPIKeyAuth>,
        requested_model: Option<&str>,
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    );
}

// B509's OpenAI config answers this through B1114's ResponsesProviderConfig
pub trait ResponsesEventTransform {
    type Error: Error + Send + Sync + 'static;

    fn transform(
        &self,
        model: &str,
        chunk: &Json,
        logging: &mut IndexMap<String, Json>,
    ) -> Result<ResponsesAPIStreamingResponse, Self::Error>;
}

#[derive(Debug, Error)]
pub enum ProcessEventError<TransformError>
where
    TransformError: Error + 'static,
{
    #[error("responses_api_provider_config is required to process live streaming chunks")]
    MissingProviderConfig,
    #[error(transparent)]
    Transform(TransformError),
    #[error(transparent)]
    Hook(#[from] PortFailure),
    #[error(transparent)]
    Rewrite(#[from] identity::RewriteError),
}

impl<TransformError> ProcessEventError<TransformError>
where
    TransformError: Error + 'static,
{
    #[must_use]
    pub fn error_class(&self, model: &str, provider: Option<&str>) -> ErrorClass {
        ErrorClass::Unclassified(Box::new(ErrorContext {
            message: self.to_string(),
            model: Some(model.to_owned()),
            provider: provider.map(str::to_owned),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        }))
    }
}

pub struct ResponsesEventProcessor<'stream, 'details, Transform, Tokenization, Calculator> {
    pub transform: Option<&'stream Transform>,
    pub tokenization: Option<&'stream Tokenization>,
    pub calculator: Option<&'stream Calculator>,
    pub hooks: &'stream ResponsesStreamHooks<'stream>,
    pub details: &'details mut IndexMap<String, Json>,
    pub dispatch: Dispatch,
    pub limits: &'stream ApiLimits,
}
