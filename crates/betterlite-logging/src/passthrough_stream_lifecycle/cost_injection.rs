use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::types::EndpointType;

const VERTEX_RAW_PREDICT: &str = "rawPredict";
const VERTEX_STREAM_RAW_PREDICT: &str = "streamRawPredict";

// LiteLLM resolves cost_injection_active once per stream: the global flag, a model name and the endpoint rules
#[must_use]
pub fn cost_injection_active(
    enabled: bool,
    model_name: Option<&str>,
    endpoint_type: EndpointType,
    url_route: &str,
) -> bool {
    if !enabled || model_name.is_none_or(str::is_empty) {
        return false;
    }
    match endpoint_type {
        EndpointType::Anthropic | EndpointType::OpenAi => true,
        EndpointType::VertexAi => {
            url_route.contains(VERTEX_STREAM_RAW_PREDICT) || url_route.contains(VERTEX_RAW_PREDICT)
        }
        EndpointType::Gemini
        | EndpointType::Tinyfish
        | EndpointType::Generic
        | EndpointType::Decisions => false,
    }
}

// ProxyBaseLLMRequestProcessing._process_chunk_with_cost_injection (B8930), supplied through B8931's seam
pub trait StreamCostInjection: Send + Sync {
    fn inject(&self, frames: &[u8], model_name: &str, logging_obj: &StreamLoggingObject)
    -> Vec<u8>;
}
