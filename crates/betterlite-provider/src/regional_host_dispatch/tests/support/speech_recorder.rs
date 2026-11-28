use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_openai::openai_base_config::Object as SpeechObject;
use secrecy::ExposeSecret as _;

use crate::api::speech_http::SpeechCall;
use crate::api::speech_provider_contract::{SpeechHeaders, TextToSpeechConfig};
use crate::regional_host_dispatch::error::DispatchError;
use crate::regional_host_dispatch::speech::SpeechHandler;

#[derive(Debug, Clone)]
pub(crate) struct SpeechSnapshot {
    pub(crate) model: String,
    pub(crate) input: String,
    pub(crate) voice: Option<String>,
    pub(crate) optional_params: SpeechObject,
    pub(crate) litellm_params: SpeechObject,
    pub(crate) extra_headers: Option<SpeechHeaders>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) ssl_verify: Option<bool>,
    pub(crate) timeout: Option<Duration>,
}

pub(crate) struct SpeechRecorder {
    calls: Mutex<Vec<SpeechSnapshot>>,
}

impl SpeechRecorder {
    pub(crate) const fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn first(&self) -> SpeechSnapshot {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .expect("the arm called the handler")
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl SpeechHandler for SpeechRecorder {
    type Reply = String;

    fn synthesize(
        &self,
        call: &SpeechCall<'_>,
        _config: &dyn TextToSpeechConfig,
    ) -> impl Future<Output = Result<Self::Reply, DispatchError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(snapshot(call));
        ready(Ok(String::from("spoken")))
    }
}

fn snapshot(call: &SpeechCall<'_>) -> SpeechSnapshot {
    SpeechSnapshot {
        model: call.model.to_owned(),
        input: call.input.to_owned(),
        voice: call.voice.map(str::to_owned),
        optional_params: call.optional_params.clone(),
        litellm_params: call.litellm_params.clone(),
        extra_headers: call.extra_headers.cloned(),
        api_key: call
            .params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        api_base: call.params.api_base.clone(),
        ssl_verify: call.params.ssl_verify,
        timeout: call.timeout,
    }
}
