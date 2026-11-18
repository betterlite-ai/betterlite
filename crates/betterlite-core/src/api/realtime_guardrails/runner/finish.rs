use std::error::Error;

use betterlite_config::sections::guardrail_params::base::OnViolation;
use futures_util::FutureExt as _;
use futures_util::future::LocalBoxFuture;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;

use super::super::callback::RealtimeGuardrailPolicy;
use super::super::error::RealtimeGuardrailsError;
use super::RealtimeGuardrails;

const LOGGED_TRANSCRIPT_CHARS: usize = 80;

type UnitFuture<'call, ConfigError, CallbackError> =
    LocalBoxFuture<'call, Result<(), RealtimeGuardrailsError<ConfigError, CallbackError>>>;

impl<Config, CallbackError> RealtimeGuardrails<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    // LiteLLM counts the violation after the frames, ends the session when the policy says so, and logs the first 80 characters
    pub(super) fn finish_block<'call>(
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

    fn close_backend(&mut self) -> UnitFuture<'_, Config::Error, CallbackError> {
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
