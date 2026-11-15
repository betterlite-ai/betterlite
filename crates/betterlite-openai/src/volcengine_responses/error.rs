use thiserror::Error;

use crate::api::responses_shapes::wire::ShapeDecodeError;

#[derive(Debug, Error)]
pub enum VolcengineResponsesError {
    #[error("api_key is required for Volcengine authentication")]
    MissingApiKey,
    #[error("a Volcengine stream event must carry a string in its type field")]
    EventType,
    #[error("the Volcengine stream event is not valid JSON: {source}")]
    EventDecode {
        #[source]
        source: ShapeDecodeError,
    },
    #[error("a Volcengine stream event must be a JSON object")]
    EventNotAnObject,
    #[error("the Volcengine stream event cannot be encoded: {source}")]
    EventEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Volcengine stream event does not match its record: {source}")]
    EventRecord {
        #[source]
        source: serde_json::Error,
    },
}
