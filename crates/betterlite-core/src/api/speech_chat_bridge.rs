pub mod arguments;
pub mod error;
pub mod response;
mod tests;

use std::error::Error;
use std::future::Future;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::chat::response::ChatResponse;
use indexmap::IndexMap;

use crate::api::speech_chat_bridge::arguments::SpeechBridgeArguments;
use crate::api::speech_chat_bridge::error::{SpeechBridgeError, SpeechTransformError};
use crate::api::speech_chat_bridge::response::{SpeechBinaryResponse, transform_response};

pub const GEMINI_TTS_CHAT_AUDIO_FORMAT: &str = "pcm16";
pub const GEMINI_TTS_RAW_RESPONSE_FORMAT: &str = "pcm";
pub const GEMINI_TTS_RESPONSE_FORMATS: [&str; 2] = ["pcm", "wav"];

// Gemini TTS detection is the two lowercase substrings, so a response model decides the conversion too
#[must_use]
pub(crate) fn is_gemini_tts_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    if !lower.contains("gemini") {
        return false;
    }
    lower.contains("tts")
}

// the projected kwargs handed to the completion pipeline; B004 owns what happens after this point
pub struct SpeechCompletionCall<'call> {
    pub kwargs: IndexMap<String, Json>,
    pub logging: &'call mut LoggingState,
}

// LiteLLM's ModelResponse as completion returns it, plus the hidden params the transformation reads
pub struct SpeechCompletionAnswer {
    pub response: ChatResponse,
    pub hidden_params: HiddenParams,
}

// LiteLLM checks the completion result's type before transforming it, and its other shapes are refused by name
pub enum SpeechCompletionOutput {
    Response(Box<SpeechCompletionAnswer>),
    Unmapped { type_name: String },
}

// B004's completion pipeline behind this seam, so the bridge never names a provider
pub trait SpeechCompletions {
    type Error: Error + Send + 'static;

    fn speech_completion(
        &self,
        call: SpeechCompletionCall<'_>,
    ) -> impl Future<Output = Result<SpeechCompletionOutput, Self::Error>> + Send;
}

// LiteLLM's SpeechToCompletionBridgeHandler, composed with the completion service instead of a module singleton
pub struct SpeechToCompletionBridge<Completions> {
    pub completions: Completions,
}

impl<Completions> SpeechToCompletionBridge<Completions>
where
    Completions: SpeechCompletions,
{
    pub async fn speech<'call>(
        &self,
        arguments: SpeechBridgeArguments<'call>,
        logging: Option<&'call mut LoggingState>,
    ) -> Result<SpeechBinaryResponse, SpeechBridgeError<Completions::Error>> {
        let validated = arguments.validate::<Completions::Error>(logging)?;
        let kwargs = validated.project()?;
        let response_format = validated.response_format();
        let logging_state = validated.logging;
        let output = self
            .completions
            .speech_completion(SpeechCompletionCall {
                kwargs,
                logging: logging_state,
            })
            .await
            .map_err(|source| SpeechBridgeError::Completion { source })?;
        match output {
            SpeechCompletionOutput::Response(answer) => {
                transform_response(*answer, response_format).map_err(SpeechBridgeError::Transform)
            }
            SpeechCompletionOutput::Unmapped { type_name } => Err(SpeechBridgeError::Transform(
                SpeechTransformError::Unmapped { type_name },
            )),
        }
    }
}
