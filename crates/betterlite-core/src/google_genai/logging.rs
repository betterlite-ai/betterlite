use std::sync::Arc;

use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::build::StreamLoggingInput;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::router::StreamLoggingRouter;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use super::metadata::GenAiStreamMetadata;

// The process-wide pieces of LiteLLM's _handle_async_streaming_logging, supplied once by the proxy
#[derive(Clone)]
pub struct GenAiStreamLogging {
    pub router: Arc<StreamLoggingRouter>,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub passthrough: Arc<PassThroughEndpointLogging>,
}

impl GenAiStreamLogging {
    // LiteLLM's _handle_async_streaming_logging hands the collected SSE bytes to the pass-through handler at EOF
    pub async fn route(
        &self,
        metadata: &GenAiStreamMetadata,
        raw_bytes: &[Vec<u8>],
        start_time: DateTime<FixedOffset>,
        end_time: DateTime<FixedOffset>,
    ) {
        let input = StreamLoggingInput {
            logging_obj: Arc::clone(&self.logging_obj),
            passthrough: Arc::clone(&self.passthrough),
            providers: Arc::clone(&self.router.providers),
            url_route: "/v1/generateContent".to_owned(),
            request_body: Some(Json::Object(metadata.request_body.clone())),
            endpoint_type: metadata.provider.endpoint_type(),
            start_time,
            end_time,
            raw_bytes: raw_bytes.to_vec(),
            model: Some(metadata.model.clone()),
        };
        self.router.route_streaming_logging_to_handler(input).await;
    }
}
