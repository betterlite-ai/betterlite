use std::collections::BTreeMap;

use betterlite_azure::images::generation::GeneratedImage;
use betterlite_azure::transcription::native::reply::NativeTranscriptionReply;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use betterlite_openai::transcription::reply::TranscriptionReply;

pub mod call;
pub mod error;
pub mod execute;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;

mod edit;
mod image;
mod parts;
pub mod runtime;
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureMediaPrepared;

// The edit's resolved request and form beside the reply its response transform reads
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEditOutcome {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub data: ImageParams,
    pub files: ImageFiles,
    pub reply: ImageGenerationReply,
}
