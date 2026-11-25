use std::sync::Arc;

use async_trait::async_trait;
use betterlite_redact::secret::Redactor;

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_reasoning_route::seams::{ModelMetadata, ProviderResolution};
use crate::api::messages_response_stream::source::ResponsesEventSource;
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::error::CreateFailure;
use crate::api::responses_create_http::service::LogContext;

use super::records::ResponsesAnswer;

// _provider_returns_encrypted_reasoning's second half: whether the provider's Responses API config lists "include" among its supported params.
pub trait ResponsesIncludeSupport {
    fn supports_include(&self, provider: &str, model: &str) -> bool;
}

// litellm.responses / litellm.aresponses behind the call this slice builds; the route supplies the implementation and its provider ports.
#[async_trait]
pub trait ResponsesCreatePort: Send + Sync {
    type Stream: ResponsesEventSource;

    async fn create(
        &self,
        call: &ResponsesCreateCall,
        log: &mut LogContext<'_, '_>,
    ) -> Result<ResponsesAnswer<Self::Stream>, CreateFailure>;
}

// The lookups _build_responses_kwargs reads: the provider's Responses config, the provider get_llm_provider infers, and the metadata the effort normalization consults.
pub struct MessagesResponseCallSeams<'seams> {
    pub include: &'seams (dyn ResponsesIncludeSupport + Send + Sync),
    pub provider: &'seams (dyn ProviderResolution + Send + Sync),
    pub metadata: &'seams (dyn ModelMetadata + Send + Sync),
    pub redactor: Option<Arc<Redactor>>,
    pub limits: ApiLimits,
}
