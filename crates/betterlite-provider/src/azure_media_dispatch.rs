use std::collections::BTreeMap;

use betterlite_azure::images::generation::GeneratedImage;
use betterlite_azure::transcription::call::reply::TranscriptionReply;
use betterlite_azure::transcription::native::reply::NativeTranscriptionReply;
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use http::Method;

pub mod call;
pub mod error;
pub mod execute;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;

mod edit;
mod image;
mod parts;
mod runtime;
mod tests;
mod transcription;

// B010's arm carries no data because OperationRequest is built before a media call picks its leaf
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureMediaRequest;

// One result per Azure media leaf; a leaf's own reply record travels through unchanged, boxed to keep the enum small
pub enum AzureMediaResult {
    ImageGeneration(Box<GeneratedImage>),
    ImageEdit(Box<ImageEditOutcome>),
    Transcription(Box<TranscriptionReply>),
    NativeTranscription(Box<NativeTranscriptionReply>),
}

// The edit's multipart request beside the reply its response transform reads
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEditOutcome {
    pub request: AzureMediaPrepared,
    pub reply: ImageGenerationReply,
}

// The multipart request the edit sends through B535's transport
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureMediaPrepared {
    pub method: Method,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub parts: Vec<MediaPart>,
}

// One multipart entry in wire order: a form field or an uploaded file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPart {
    pub field: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}
