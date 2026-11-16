use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::image_generation::request::ImageGenerationOutcome;
use betterlite_openai::image_variations::request::ImageVariationOutcome;
use betterlite_openai::speech::result::SpeechResponse;
use betterlite_openai::transcription::reply::TranscriptionReply;
use betterlite_openai::video_edits::response::CharacterObject;
use betterlite_openai::video_edits::response::VideoObject as EditedVideo;
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::list::VideoList;
use betterlite_openai::videos::reply::VideoObject;
use indexmap::IndexMap;

pub mod call;
pub mod error;
pub mod execute;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;
pub mod video;
pub mod video_edit;

mod parts;

mod tests;

// B010's arm carries no data because OperationRequest is built before a media call picks its leaf
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiMediaRequest;

// One result per media leaf; a leaf's own reply record travels through unchanged, boxed to keep the enum small
pub enum OpenAiMediaResult {
    ImageGeneration(Box<ImageGenerationOutcome>),
    ImageEdit(Box<OpenAiMediaPrepared>),
    ImageVariation(Box<ImageVariationOutcome>),
    Speech(Box<SpeechResponse>),
    Transcription(Box<TranscriptionReply>),
    Video(Box<VideoOutcome>),
    VideoEdit(Box<VideoEditOutcome>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaMethod {
    Get,
    Post,
    Delete,
}

// The request a leaf without a transport prepares for the route to send; the payload keeps the leaf's own data type.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiMediaPrepared {
    pub method: MediaMethod,
    pub url: String,
    pub query: VideoOptions,
    pub body: MediaPayload,
    pub parts: Vec<MediaPart>,
    pub multipart: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaPayload {
    Empty,
    // B512's edit form data, which the edit sends beside its files
    ImageData(ImageParams),
    // B525's create fields, its list parameters and its remix body
    Options(VideoOptions),
    // B526's edit and extension bodies
    Body(IndexMap<String, Json>),
}

// One multipart entry in the order the leaf builds it: a plain text field or an uploaded file.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPart {
    pub field: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub body: MediaPartBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaPartBody {
    Bytes(Vec<u8>),
    Text(String),
    // a caller-held reader the leaf was handed as a stream; the composition keeps no copy of it
    Streamed,
}

pub struct VideoOutcome {
    pub request: OpenAiMediaPrepared,
    pub reply: VideoReplyKind,
}

pub enum VideoReplyKind {
    Video(Box<VideoObject>),
    List(Box<VideoList>),
    Bytes(Vec<u8>),
}

pub struct VideoEditOutcome {
    pub request: OpenAiMediaPrepared,
    pub reply: VideoEditReplyKind,
}

pub enum VideoEditReplyKind {
    Video(Box<EditedVideo>),
    Character(Box<CharacterObject>),
}
