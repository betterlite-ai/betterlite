use betterlite_streaming::stream_content_assembly::AssemblyError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VeoOperationError {
    #[error("the Veo operation response is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the Veo operation response is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("the Veo operation response value could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("No operation name in Veo response")]
    MissingName,
    #[error("No operation name in Veo edit response")]
    MissingEditName,
    #[error(
        "Invalid operation name format: {operation}. Expected format: projects/PROJECT/locations/LOCATION/publishers/google/models/MODEL/operations/OPERATION_ID"
    )]
    InvalidOperation { operation: String },
    #[error("the request parameters are not an object")]
    ParametersShape,
    #[error("No video data found in completed operation")]
    NoVideos,
    #[error(
        "Video generation is not complete yet. Please check status with video_status() before downloading."
    )]
    NotComplete,
    #[error("No base64 encoded video data found")]
    NoBase64,
    #[error("the video content is not valid base64")]
    VideoBase64(#[source] AssemblyError),
    #[error("{message}")]
    Unsupported { message: &'static str },
}
