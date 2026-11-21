use std::future::Future;
use std::future::ready;

use betterlite_context::media_config_selection::audio_transcription::AudioTranscriptionConfigTag;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_models::call_helpers::headers::ProviderHeaderProjection;
use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;

use crate::dispatch::error::DispatchError;

use super::error::{TranscriptionDispatchError, TranscriptionError};
use super::target::{InjectedTranscription, TranscriptionTarget};

// what a provider operation hands back: the typed response and the headers the reply carried
pub struct TranscriptionProviderReply {
    pub response: TranscriptionResponse,
    pub provider_headers: ProviderHeaderProjection,
}

// transcription's runtime union: a typed reply, a cached dict, or anything else a mocked handler returns
pub enum TranscriptionAnswer {
    Reply(Box<TranscriptionProviderReply>),
    Mapping(Json),
    Unexpected(Json),
}

// the seam a downstream provider implementation fills; B515/B539/B2453 keep their own transports
pub trait TranscriptionDispatch: Send + Sync {
    type Error: TranscriptionDispatchError;

    fn transcribe(
        &self,
        target: TranscriptionTarget<'_>,
    ) -> impl Future<Output = Result<TranscriptionAnswer, Self::Error>> + Send;

    // the provider config's own param mapper, handed to get_optional_params_transcription
    fn mapper(
        &self,
        _provider: &str,
        _config: Option<AudioTranscriptionConfigTag>,
    ) -> Option<&dyn OptionalParamsMapper> {
        None
    }

    // IBMWatsonX runs an injected operation; with none injected main's stub answers NotBuilt
    fn watsonx(
        &self,
        _target: InjectedTranscription<'_>,
    ) -> impl Future<Output = Result<TranscriptionAnswer, TranscriptionError<Self::Error>>> + Send
    {
        ready(Err(TranscriptionError::NotBuilt(DispatchError::NotBuilt {
            entry: "watsonx_transcription",
        })))
    }
}
