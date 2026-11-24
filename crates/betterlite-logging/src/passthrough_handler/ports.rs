use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use rust_decimal::Decimal;

use super::types::{PassthroughHandlerError, StreamedResponse, UpstreamChatResponse};
use crate::passthrough_stream_logging::object::StreamLoggingObject;

// LiteLLM's BaseConfig.transform_response, cut down to the arguments the base handler passes
pub trait BaseConfig: Send + Sync {
    fn transform_response(
        &self,
        upstream: UpstreamChatResponse<'_>,
        model: &str,
        logging_obj: &StreamLoggingObject,
    ) -> Result<Json, PassthroughHandlerError>;
}

// the abstract members of LiteLLM's BasePassthroughLoggingHandler, implemented by a provider leaf
pub trait PassthroughLoggingProvider: Send + Sync {
    fn llm_provider_name(&self) -> ProviderId;

    fn get_provider_config(&self, model: &str) -> Box<dyn BaseConfig>;

    fn build_streaming_response(
        &self,
        all_chunks: &[String],
        logging_obj: &StreamLoggingObject,
        model: &str,
    ) -> Result<Option<StreamedResponse>, PassthroughHandlerError>;
}

// B068's completion_cost, which decides whether an upstream-reported cost wins
pub trait ResponseCost: Send + Sync {
    fn completion_cost(
        &self,
        response: &Json,
        model: &str,
    ) -> Result<Decimal, PassthroughHandlerError>;
}
