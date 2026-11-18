mod block;
mod finish;

use std::convert::Infallible;
use std::error::Error;
use std::ptr::{eq, from_ref};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use futures_util::future::LocalBoxFuture;
use indexmap::IndexMap;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::TRANSCRIPTION_EVENT_HOOKS;
use crate::api::realtime_session_state::RealtimeStreaming;
use crate::api::request_destination::RequestData;

use super::callback::{GuardrailFailure, GuardrailInput, GuardrailInputType, RealtimeGuardrail};
use super::error::RealtimeGuardrailsError;
use super::message;

// The private steps stay local futures: the connection's callbacks and sockets are not Send, like B1110's sender
type BlockFuture<'call, ConfigError, CallbackError> =
    LocalBoxFuture<'call, Result<bool, RealtimeGuardrailsError<ConfigError, CallbackError>>>;

// LiteLLM's RealTimeStreaming.run_realtime_guardrails: the connection state, the provider config and litellm.callbacks
pub struct RealtimeGuardrails<'run, Config, CallbackError = Infallible>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    pub state: &'run mut RealtimeStreaming,
    pub provider: Option<&'run Config>,
    pub guardrails: &'run [&'run dyn RealtimeGuardrail<Error = CallbackError>],
}

// The relay runs it on the connection state it already owns; a block sends its synthetic frames through B1110's boundary
#[async_trait(?Send)]
pub trait RunRealtimeGuardrails {
    type Error;

    async fn run_realtime_guardrails(
        &mut self,
        transcript: &str,
        item_id: Option<&str>,
        pre_block_backend_message: Option<&str>,
        event_hooks: Option<&[GuardrailEventHooks]>,
    ) -> Result<bool, Self::Error>;
}

#[async_trait(?Send)]
impl<Config, CallbackError> RunRealtimeGuardrails for RealtimeGuardrails<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    type Error = RealtimeGuardrailsError<Config::Error, CallbackError>;

    async fn run_realtime_guardrails(
        &mut self,
        transcript: &str,
        _item_id: Option<&str>,
        pre_block_backend_message: Option<&str>,
        event_hooks: Option<&[GuardrailEventHooks]>,
    ) -> Result<bool, Self::Error> {
        let hooks = event_hooks.unwrap_or(&TRANSCRIPTION_EVENT_HOOKS);
        let check_data = self.check_data(transcript);
        let guardrails = self.guardrails;
        let mut already_run = Vec::new();
        for &guardrail in guardrails {
            if !Self::eligible(guardrail, &already_run, &check_data, hooks)? {
                continue;
            }
            already_run.push(from_ref(guardrail).cast::<()>());
            if self
                .apply_and_block(guardrail, pre_block_backend_message, transcript)
                .await?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl<Config, CallbackError> RealtimeGuardrails<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    // LiteLLM's _check_data: the request data the relay already holds, with the transcript replacing any key of that name
    fn check_data(&self, transcript: &str) -> RequestData {
        let mut data = self.state.request_data.clone();
        drop(data.insert(
            String::from("transcript"),
            Json::Text(transcript.to_owned()),
        ));
        data
    }

    // LiteLLM skips a non-guardrail, a native-lifecycle guardrail, an object that already ran, and one no hook selects
    fn eligible(
        guardrail: &dyn RealtimeGuardrail<Error = CallbackError>,
        already_run: &[*const ()],
        check_data: &RequestData,
        hooks: &[GuardrailEventHooks],
    ) -> Result<bool, RealtimeGuardrailsError<Config::Error, CallbackError>> {
        if !guardrail.is_custom_guardrail() || guardrail.use_native_lifecycle_hooks() {
            return Ok(false);
        }
        let identity = from_ref(guardrail).cast::<()>();
        if already_run.iter().any(|seen| eq(*seen, identity)) {
            return Ok(false);
        }
        for hook in hooks {
            if guardrail
                .should_run_guardrail(check_data, *hook)
                .map_err(RealtimeGuardrailsError::Predicate)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // The first block ends the run; a clean apply drops its rewritten texts, as LiteLLM does
    fn apply_and_block<'call>(
        &'call mut self,
        guardrail: &'call dyn RealtimeGuardrail<Error = CallbackError>,
        pre_block_backend_message: Option<&'call str>,
        transcript: &'call str,
    ) -> BlockFuture<'call, Config::Error, CallbackError> {
        async move {
            let input = GuardrailInput {
                texts: vec![transcript.to_owned()],
                images: Vec::new(),
                request_data: IndexMap::from([(
                    String::from("user_api_key_dict"),
                    self.state.user_api_key_dict.clone().unwrap_or(Json::Null),
                )]),
                input_type: GuardrailInputType::Request,
            };
            match guardrail.apply_guardrail(input).await {
                Ok(_) => Ok(false),
                Err(GuardrailFailure::Unexpected(error)) => {
                    tracing::error!(
                        "[realtime guardrail] unexpected error in apply_guardrail: {error}"
                    );
                    Err(RealtimeGuardrailsError::Unexpected(error))
                }
                Err(GuardrailFailure::Blocked(violation)) => {
                    let policy = guardrail.policy();
                    let message =
                        message::violation_message(&violation, policy.violation_message.as_deref());
                    block::send_block_frames(&mut *self, &message, pre_block_backend_message)
                        .await?;
                    finish::finish_block(self, policy, transcript).await?;
                    Ok(true)
                }
            }
        }
        .boxed_local()
    }
}
