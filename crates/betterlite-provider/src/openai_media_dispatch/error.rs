use betterlite_openai::image_edits::error::ImageEditError;
use betterlite_openai::image_generation::error::ImageGenerationFailure;
use betterlite_openai::image_variations::error::OpenAiImageVariationError;
use betterlite_openai::speech::error::SpeechError;
use betterlite_openai::transcription::error::TranscriptionError;
use betterlite_openai::video_edits::error::VideoEditError;
use betterlite_openai::videos::error::VideoError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenAiMediaError {
    #[error(transparent)]
    ImageGeneration(#[from] ImageGenerationFailure),
    #[error(transparent)]
    ImageEdit(#[from] ImageEditError),
    #[error(transparent)]
    ImageVariation(#[from] OpenAiImageVariationError),
    #[error(transparent)]
    Speech(#[from] SpeechError),
    #[error(transparent)]
    Transcription(#[from] TranscriptionError),
    #[error(transparent)]
    Video(#[from] VideoError),
    #[error(transparent)]
    VideoEdit(#[from] VideoEditError),
    #[error(
        "the openai media composition runs only image, speech, transcription and video operations"
    )]
    UnsupportedOperation,
}
