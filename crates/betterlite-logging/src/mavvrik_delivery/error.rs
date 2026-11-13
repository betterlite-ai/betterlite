use std::io;

use thiserror::Error;

use super::validation::ValidationError;

#[derive(Debug, Error)]
pub enum MavvrikDeliveryError<TransportFailure> {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("Mavvrik FOCUS destination: the request failed")]
    Transport {
        #[source]
        source: TransportFailure,
    },
    #[error(
        "Mavvrik FOCUS destination: connector is disconnected (410). Re-enable the connection in the Mavvrik dashboard."
    )]
    Disconnected,
    #[error("Mavvrik FOCUS destination: register failed ({status}): {body}")]
    RegisterFailed { status: u16, body: String },
    #[error("Mavvrik FOCUS destination: failed to get signed URL ({status}): {body}")]
    SignedUrlFailed { status: u16, body: String },
    #[error("Mavvrik FOCUS destination: response missing 'url' field")]
    SignedUrlMissing,
    #[error("Mavvrik FOCUS destination: failed to update metricsMarker ({status}): {body}")]
    MarkerUpdateFailed { status: u16, body: String },
    #[error("Mavvrik FOCUS destination: GCS session init failed ({status}): {body}")]
    SessionInitFailed { status: u16, body: String },
    #[error("Mavvrik FOCUS destination: GCS session init missing Location header")]
    SessionLocationMissing,
    #[error(
        "Mavvrik FOCUS destination: GCS chunk upload failed (chunk offset={offset}, expected={expected}, got={status}): {body}"
    )]
    ChunkFailed {
        offset: usize,
        expected: &'static str,
        status: u16,
        body: String,
    },
    #[error("Mavvrik FOCUS destination: could not build the request URL")]
    RequestUrl(#[source] url::ParseError),
    #[error("Mavvrik FOCUS destination: the JSON body could not be read or written")]
    Json(#[source] serde_json::Error),
    #[error("Mavvrik FOCUS destination: could not gzip the export")]
    Compress(#[source] io::Error),
}
