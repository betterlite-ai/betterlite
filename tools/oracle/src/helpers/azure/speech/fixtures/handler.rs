use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::time::Duration;

use betterlite_azure::speech::dispatch::{HandlerFailure, HandlerRequest, TextToSpeechHandler};
use betterlite_azure::speech::logging::{SpeechLogger, SpeechPreCall};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) pre: Option<PreCall>,
}

pub(crate) struct PreCall {
    pub(crate) input: String,
    pub(crate) api_base: String,
    pub(crate) data: IndexMap<String, Json>,
}

impl SpeechLogger for Recorder {
    fn pre_call(&mut self, record: SpeechPreCall<'_>) -> Result<(), PrePostError> {
        self.pre = Some(PreCall {
            input: record.input.to_owned(),
            api_base: record.api_base.to_owned(),
            data: record.data,
        });
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct HandlerRecord {
    pub(crate) voice: Option<String>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) provider: String,
    pub(crate) is_async: bool,
}

// The answer stands in for the audio bytes the provider handler returns; the oracle case only compares what dispatch passes.
#[derive(Default)]
pub(crate) struct RecordingHandler {
    pub(crate) requests: Vec<HandlerRecord>,
}

pub(crate) const HANDLER_ANSWER: &str = "handler-answer";

impl TextToSpeechHandler for RecordingHandler {
    type Output = String;

    fn text_to_speech(
        &mut self,
        request: HandlerRequest<'_>,
    ) -> impl Future<Output = Result<String, HandlerFailure>> + Send {
        self.requests.push(HandlerRecord {
            voice: request.voice.map(str::to_owned),
            optional_params: request.optional_params.clone(),
            litellm_params: request.litellm_params.clone(),
            extra_headers: request.extra_headers.cloned(),
            timeout: request.timeout,
            provider: request.provider.to_owned(),
            is_async: request.is_async,
        });
        ready(Ok(String::from(HANDLER_ANSWER)))
    }
}
