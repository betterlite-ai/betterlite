use std::convert::Infallible;
use std::error::Error;
use std::ptr::{eq, from_ref};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_params::base::OnViolation;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use futures_util::future::LocalBoxFuture;
use indexmap::IndexMap;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::{RealtimeGuardrailCallback, TRANSCRIPTION_EVENT_HOOKS};
use crate::api::realtime_sender::RealtimeSend as _;
use crate::api::realtime_sender::RealtimeSender;
use crate::api::realtime_session_state::RealtimeStreaming;
use crate::api::request_destination::RequestData;

use super::callback::{
    GuardrailFailure, GuardrailInput, GuardrailInputType, GuardrailTexts, RealtimeGuardrail,
    RealtimeGuardrailPolicy,
};
use super::error::RealtimeGuardrailsError;
use super::message;

const LOGGED_TRANSCRIPT_CHARS: usize = 80;

// The private steps stay local futures: the connection's callbacks and sockets are not Send, like B1110's sender
type BlockFuture<'call, ConfigError, CallbackError> =
    LocalBoxFuture<'call, Result<bool, RealtimeGuardrailsError<ConfigError, CallbackError>>>;
type UnitFuture<'call, ConfigError, CallbackError> =
    LocalBoxFuture<'call, Result<(), RealtimeGuardrailsError<ConfigError, CallbackError>>>;
type ApplyFuture<'call, CallbackError> =
    LocalBoxFuture<'call, Result<GuardrailTexts, GuardrailFailure<CallbackError>>>;

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
            match self.apply(guardrail, transcript).await {
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
                    self.send_block_frames(&message, pre_block_backend_message)
                        .await?;
                    self.finish_block(policy, transcript).await?;
                    Ok(true)
                }
            }
        }
        .boxed_local()
    }

    fn apply<'call>(
        &'call self,
        guardrail: &'call dyn RealtimeGuardrail<Error = CallbackError>,
        transcript: &'call str,
    ) -> ApplyFuture<'call, CallbackError> {
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
            guardrail.apply_guardrail(input).await
        }
        .boxed_local()
    }

    // The frame order: the caller's protocol frame first, the cancellation, the client notice, then the spoken message and a fresh response
    fn send_block_frames<'call>(
        &'call mut self,
        message: &'call Json,
        pre_block_backend_message: Option<&'call str>,
    ) -> UnitFuture<'call, Config::Error, CallbackError> {
        async move {
            let (cancel, response_create) = message::control_frames();
            if let Some(frames) = pre_block_backend_message {
                self.send_backend_text(frames).await?;
            }
            let cancel_text =
                serde_json::to_string(&cancel).map_err(RealtimeGuardrailsError::Encode)?;
            self.send_backend_text(&cancel_text).await?;
            self.send_client_notice(message).await?;
            let spoken_text = serde_json::to_string(&message::spoken_message(message))
                .map_err(RealtimeGuardrailsError::Encode)?;
            self.send_backend_text(&spoken_text).await?;
            let create_text =
                serde_json::to_string(&response_create).map_err(RealtimeGuardrailsError::Encode)?;
            self.send_backend_text(&create_text).await
        }
        .boxed_local()
    }

    fn finish_block<'call>(
        &'call mut self,
        policy: &'call RealtimeGuardrailPolicy,
        transcript: &'call str,
    ) -> UnitFuture<'call, Config::Error, CallbackError> {
        async move {
            self.state.setup.violation_count = self.state.setup.violation_count.saturating_add(1);
            let count = self.state.setup.violation_count;
            // LiteLLM's int | None; a count past i64::MAX is past every threshold a config can hold
            let comparable = i64::try_from(count).unwrap_or(i64::MAX);
            let limit_reached = policy
                .end_session_after_n_fails
                .is_some_and(|limit| comparable >= limit);
            if policy.on_violation == Some(OnViolation::EndSession) || limit_reached {
                tracing::warn!("[realtime guardrail] ending session after violation {count}");
                self.close_backend().await?;
            }
            let logged: String = transcript.chars().take(LOGGED_TRANSCRIPT_CHARS).collect();
            tracing::warn!(
                "[realtime guardrail] BLOCKED transcript (violation {count}): {logged:?}"
            );
            Ok(())
        }
        .boxed_local()
    }

    // B1110's send boundary: a guardrail frame gets the same provider transform as a client frame
    fn send_backend_text<'call>(
        &'call mut self,
        text: &'call str,
    ) -> UnitFuture<'call, Config::Error, CallbackError> {
        async move {
            let guardrails: &[&dyn RealtimeGuardrailCallback<Error = CallbackError>] = &[];
            let mut sender = RealtimeSender {
                state: &mut *self.state,
                provider: self.provider,
                guardrails,
            };
            sender
                .send_to_backend(text)
                .await
                .map_err(RealtimeGuardrailsError::Send)?;
            Ok(())
        }
        .boxed_local()
    }

    // LiteLLM sends the notice straight to the client socket; it is not an event, so it gets no normalization or beta translation
    fn send_client_notice<'call>(
        &'call mut self,
        message: &'call Json,
    ) -> UnitFuture<'call, Config::Error, CallbackError> {
        async move {
            let error = Json::Object(IndexMap::from([
                (
                    String::from("type"),
                    Json::Text(String::from("guardrail_violation")),
                ),
                (String::from("message"), message.clone()),
                (
                    String::from("code"),
                    Json::Text(String::from("content_policy_violation")),
                ),
            ]));
            let notice = Json::Object(IndexMap::from([
                (String::from("type"), Json::Text(String::from("error"))),
                (String::from("error"), error),
            ]));
            let text = serde_json::to_string(&notice).map_err(RealtimeGuardrailsError::Encode)?;
            let client = self
                .state
                .client
                .as_mut()
                .ok_or(RealtimeGuardrailsError::ClientMissing)?;
            client
                .send_text(&text)
                .await
                .map_err(RealtimeGuardrailsError::Client)
        }
        .boxed_local()
    }

    fn close_backend<'call>(&'call mut self) -> UnitFuture<'call, Config::Error, CallbackError> {
        async move {
            let backend = self
                .state
                .backend
                .as_mut()
                .ok_or(RealtimeGuardrailsError::BackendMissing)?;
            backend
                .close()
                .await
                .map_err(RealtimeGuardrailsError::Close)
        }
        .boxed_local()
    }
}
