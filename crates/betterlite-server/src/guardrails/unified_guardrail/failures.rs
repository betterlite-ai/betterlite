use betterlite_core::passthrough_guardrail::errors::PassthroughGuardrailError;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;

use crate::guardrails::a2a_translation::A2aTranslationError;
use crate::guardrails::anthropic_translation::output_scan::error::ProcessOutputError;
use crate::guardrails::anthropic_translation::process_input::error::ProcessInputError;
use crate::guardrails::anthropic_translation::process_stream::error::ProcessStreamError;
use crate::guardrails::bedrock_passthrough::output::OutputTranslationError;
use crate::guardrails::bedrock_passthrough::request::RequestTranslationError;
use crate::guardrails::chat_guardrail_translation::rewrites::error::{
    EndedStreamError, InputError,
};
use crate::guardrails::completion_translation::CompletionTranslationError;
use crate::guardrails::embeddings_translation::EmbeddingsTranslationError;
use crate::guardrails::image_translation::ImageTranslationError;
use crate::guardrails::responses_translation::output_rewrite::error::OutputRewriteError;
use crate::guardrails::responses_translation::request_rewrite::error::RequestRewriteError;
use crate::guardrails::responses_translation::stream_output::error::StreamOutputError;
use crate::guardrails::speech_translation::SpeechTranslationError;
use crate::guardrails::transcription_translation::TranscriptionTranslationError;
use crate::guardrails::unified_chat::anthropic::AnthropicTranslationError;
use crate::guardrails::unified_chat::chat::ChatTranslationError;
use crate::guardrails::unified_chat::responses::ResponsesTranslationError;
use crate::guardrails::unified_passthrough::bedrock::BedrockPassthroughError;
use crate::guardrails::video_translation::VideoTranslationError;

// The unified hooks and B6164's stream loop speak GuardrailFailure; an adapter's own parse error becomes Other
pub trait AdapterFailure: Sized {
    fn into_failure(self) -> GuardrailFailure;
    fn from_failure(failure: GuardrailFailure) -> Self;
}

impl AdapterFailure for GuardrailFailure {
    fn into_failure(self) -> GuardrailFailure {
        self
    }

    fn from_failure(failure: GuardrailFailure) -> Self {
        failure
    }
}

macro_rules! guardrail_only {
    ($($error:ty),+) => {
        $(impl AdapterFailure for $error {
            fn into_failure(self) -> GuardrailFailure {
                match self {
                    Self::Guardrail(failure) => failure,
                }
            }

            fn from_failure(failure: GuardrailFailure) -> Self {
                Self::Guardrail(failure)
            }
        })+
    };
}

guardrail_only!(
    CompletionTranslationError,
    ImageTranslationError,
    SpeechTranslationError,
    TranscriptionTranslationError,
    VideoTranslationError
);

// the guardrail arms give the failure back; every other error becomes Other with its text
macro_rules! guardrail_or_other {
    ($error:ty, $($found:pat => $value:expr),+ ; other $rest:pat, back $failure:ident => $back:expr) => {
        impl AdapterFailure for $error {
            fn into_failure(self) -> GuardrailFailure {
                match self {
                    $($found => $value,)+
                    other @ ($rest) => GuardrailFailure::Other(other.to_string()),
                }
            }

            fn from_failure($failure: GuardrailFailure) -> Self {
                $back
            }
        }
    };
}

guardrail_or_other!(
    EmbeddingsTranslationError,
    Self::Guardrail(failure) => failure;
    other Self::NonTextItem { .. },
    back failure => Self::Guardrail(failure)
);

guardrail_or_other!(
    A2aTranslationError,
    Self::Guardrail(failure) => failure;
    other Self::Text(_) | Self::Json { .. },
    back failure => Self::Guardrail(failure)
);

guardrail_or_other!(
    ChatTranslationError,
    Self::Guardrail(failure)
        | Self::Input(InputError::Guardrail(failure))
        | Self::Stream(EndedStreamError::Guardrail(failure)) => failure;
    other Self::Input(_) | Self::Stream(_) | Self::Parse { .. },
    back failure => Self::Guardrail(failure)
);

guardrail_or_other!(
    ResponsesTranslationError,
    Self::Request(RequestRewriteError::Guardrail { source })
        | Self::Output(OutputRewriteError::Guardrail { source }) => *source,
    Self::Stream(StreamOutputError::Guardrail(failure)) => failure;
    other Self::Request(_) | Self::Output(_) | Self::Stream(_) | Self::Undeliverable(_) | Self::Json(_),
    back failure => Self::Stream(StreamOutputError::Guardrail(failure))
);

guardrail_or_other!(
    AnthropicTranslationError,
    Self::Input(ProcessInputError::Guardrail { source })
        | Self::Output(ProcessOutputError::Guardrail { source }) => *source,
    Self::Stream(ProcessStreamError::Guardrail(failure)) => failure;
    other Self::Input(_) | Self::Output(_) | Self::Stream(_),
    back failure => Self::Stream(ProcessStreamError::Guardrail(failure))
);

guardrail_or_other!(
    PassthroughGuardrailError<GuardrailFailure>,
    Self::Guardrail(failure) => failure;
    other Self::Payload(_),
    back failure => Self::Guardrail(failure)
);

impl AdapterFailure for BedrockPassthroughError {
    fn into_failure(self) -> GuardrailFailure {
        match self {
            Self::Request(RequestTranslationError::Guardrail(failure))
            | Self::Output(OutputTranslationError::Guardrail(failure)) => failure,
            Self::Endpoint(endpoint) => endpoint.into_failure(),
        }
    }

    fn from_failure(failure: GuardrailFailure) -> Self {
        Self::Endpoint(PassthroughGuardrailError::Guardrail(failure))
    }
}
