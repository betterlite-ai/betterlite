use std::fmt::Display;

use betterlite_models::decisions::Json;
use betterlite_openai::speech::error::SpeechError;
use betterlite_openai::speech::request::SpeechOptions;
use betterlite_openai::transcription::error::TranscriptionError;
use betterlite_openai::videos::create::VideoCreate;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::openai_media_dispatch::OpenAiMediaRequest;
use betterlite_provider::openai_media_dispatch::call::OpenAiMediaOperation;
use betterlite_provider::openai_media_dispatch::error::OpenAiMediaError;
use betterlite_provider::openai_media_dispatch::execute::dispatch;
use betterlite_provider::operation::{OperationRequest, OperationResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Builder;

mod fixtures;
mod image;
mod tests;
mod video;
mod views;
mod voice;

pub(crate) struct Op;

// The oracle calls one operation per input; `case` names it the way the Python adapter does
#[derive(Debug, Deserialize)]
#[serde(tag = "case", rename_all = "snake_case")]
pub(super) enum Input {
    ImageGeneration(ImageGenerationCase),
    ImageEdit(ImageEditCase),
    ImageVariation(ImageVariationCase),
    Speech(SpeechCase),
    Transcription(TranscriptionCase),
    VideoCreate(VideoCreateCase),
    VideoLifecycle(VideoLifecycleCase),
    VideoExtendedOperations(VideoExtendedCase),
}

#[derive(Debug, Deserialize)]
pub(super) struct ImageGenerationCase {
    pub(super) api_base: String,
    pub(super) model: String,
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
    pub(super) status: u16,
    pub(super) reply: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct ImageEditCase {
    pub(super) api_base: String,
    pub(super) model: String,
    pub(super) prompt: Option<String>,
    pub(super) image: Vec<ImageFileCase>,
    pub(super) mask: Option<Vec<ImageFileCase>>,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ImageFileCase {
    #[serde(default)]
    pub(super) name: Option<String>,
    pub(super) content: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ImageVariationCase {
    pub(super) api_base: String,
    pub(super) model: String,
    pub(super) image_filename: Option<String>,
    pub(super) image_content: String,
    pub(super) image_content_type: Option<String>,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
    pub(super) status: u16,
    pub(super) reply: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpeechCase {
    pub(super) api_base: String,
    pub(super) model: String,
    pub(super) input: String,
    pub(super) voice: String,
    #[serde(default)]
    pub(super) options: SpeechOptions,
    pub(super) status: u16,
    pub(super) content_type: Option<String>,
    pub(super) body: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TranscriptionFamilyCase {
    Whisper,
    Gpt,
}

#[derive(Debug, Deserialize)]
pub(super) struct TranscriptionCase {
    pub(super) api_base: String,
    pub(super) model: String,
    pub(super) family: TranscriptionFamilyCase,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
    pub(super) status: u16,
    pub(super) reply: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct VideoCreateCase {
    pub(super) api_base: String,
    pub(super) model: String,
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) options: VideoCreate,
    pub(super) reference: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) reply: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VideoLifecycleOp {
    List,
    Status,
    Delete,
    Content,
    Remix,
}

#[derive(Debug, Deserialize)]
pub(super) struct VideoLifecycleCase {
    pub(super) api_base: String,
    pub(super) provider: Option<String>,
    pub(super) op: VideoLifecycleOp,
    pub(super) video_id: Option<String>,
    pub(super) after: Option<String>,
    pub(super) limit: Option<i64>,
    pub(super) order: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) prompt: Option<String>,
    pub(super) reply: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VideoExtendedOp {
    Edit,
    Extension,
    CreateCharacter,
    GetCharacter,
}

#[derive(Debug, Deserialize)]
pub(super) struct VideoExtendedCase {
    pub(super) api_base: String,
    pub(super) provider: Option<String>,
    pub(super) model: String,
    pub(super) op: VideoExtendedOp,
    pub(super) prompt: Option<String>,
    pub(super) video_id: Option<String>,
    pub(super) video_file: Option<String>,
    pub(super) extra_body: Option<IndexMap<String, Json>>,
    pub(super) seconds: Option<String>,
    pub(super) name: Option<String>,
    pub(super) video: Option<String>,
    pub(super) character_id: Option<String>,
    pub(super) reply: String,
}

pub(super) fn failure<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

// Reached only if an operation and the result variant it reads back ever disagree
pub(super) const OTHER_OUTCOME: &str = "the dispatch returned another outcome";

// A leaf failure the input never meant to raise; the rows that do raise one project it themselves
pub(super) fn leaf_failure(error: &OpenAiMediaError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("LeafFailure"),
        status: leaf_status(error),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn leaf_status(error: &OpenAiMediaError) -> Option<u16> {
    match error {
        OpenAiMediaError::ImageGeneration(raised) => raised.status(),
        OpenAiMediaError::ImageVariation(raised) => Some(raised.status_code),
        OpenAiMediaError::ImageEdit(raised) => raised.status(),
        OpenAiMediaError::Speech(SpeechError::Status(raised)) => Some(raised.status),
        OpenAiMediaError::Transcription(TranscriptionError::Status(raised)) => Some(raised.status),
        OpenAiMediaError::Speech(_)
        | OpenAiMediaError::Transcription(_)
        | OpenAiMediaError::Video(_)
        | OpenAiMediaError::VideoEdit(_)
        | OpenAiMediaError::UnsupportedOperation => None,
    }
}

// Every row runs the same production dispatch entry against its own fake transport
pub(super) async fn run(
    fixture: &fixtures::Fixture,
    operation: OpenAiMediaOperation,
) -> Result<OperationResult, OpenAiMediaError> {
    let mut logger = fixtures::Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        operation,
        fixture,
        &mut logger,
    )
    .await
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(dispatch_input(input))
    }
}

async fn dispatch_input(input: &Input) -> HelperResult {
    match input {
        Input::ImageGeneration(case) => image::image_generation(case).await,
        Input::ImageEdit(case) => image::image_edit(case).await,
        Input::ImageVariation(case) => image::image_variation(case).await,
        Input::Speech(case) => voice::speech(case).await,
        Input::Transcription(case) => voice::transcription(case).await,
        Input::VideoCreate(case) => video::video_create(case).await,
        Input::VideoLifecycle(case) => video::video_lifecycle(case).await,
        Input::VideoExtendedOperations(case) => video::video_extended(case).await,
    }
}
