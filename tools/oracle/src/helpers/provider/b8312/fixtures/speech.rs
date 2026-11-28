use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::api::speech_http::SpeechCall;
use betterlite_provider::api::speech_provider_contract::TextToSpeechConfig;
use betterlite_provider::regional_host_dispatch::error::DispatchError;
use betterlite_provider::regional_host_dispatch::speech::SpeechHandler;
use serde_json::{Value, json};

use super::calls::input_failure;

#[derive(Default)]
pub(crate) struct SpeechRecorder {
    view: Mutex<Option<Result<Value, String>>>,
}

impl SpeechRecorder {
    pub(crate) fn take_view(&self) -> Result<Value, Box<HelperError>> {
        let taken = self
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match taken {
            Some(Ok(view)) => Ok(view),
            Some(Err(error)) => Err(input_failure(error)),
            None => Err(input_failure(String::from(
                "the speech arm called no handler",
            ))),
        }
    }
}

impl SpeechHandler for SpeechRecorder {
    type Reply = ();

    fn synthesize(
        &self,
        call: &SpeechCall<'_>,
        _config: &dyn TextToSpeechConfig,
    ) -> impl Future<Output = Result<Self::Reply, DispatchError>> + Send {
        let view = json!({"voice": call.voice, "params": call.optional_params});
        *self.view.lock().unwrap_or_else(PoisonError::into_inner) = Some(Ok(view));
        ready(Ok(()))
    }
}
