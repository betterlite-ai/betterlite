use std::error::Error;

use betterlite_config::sections::guardrail_params::base::OnViolation;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;

use super::super::callback::RealtimeGuardrailPolicy;
use super::super::error::RealtimeGuardrailsError;
use super::RealtimeGuardrails;

const LOGGED_TRANSCRIPT_CHARS: usize = 80;

type UnitFuture<'call, ConfigError, CallbackError> =
    BoxFuture<'call, Result<(), RealtimeGuardrailsError<ConfigError, CallbackError>>>;

// LiteLLM counts the violation after the frames, ends the session when the policy says so, and logs the first 80 characters
pub(super) fn finish_block<'call, Config, CallbackError>(
    runner: &'call mut RealtimeGuardrails<'_, Config, CallbackError>,
    policy: &'call RealtimeGuardrailPolicy,
    transcript: &'call str,
) -> UnitFuture<'call, Config::Error, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async move {
        runner.state.setup.violation_count = runner.state.setup.violation_count.saturating_add(1);
        let count = runner.state.setup.violation_count;
        // LiteLLM's int | None; a count past i64::MAX is past every threshold a config can hold
        let comparable = i64::try_from(count).unwrap_or(i64::MAX);
        let limit_reached = policy
            .end_session_after_n_fails
            .is_some_and(|limit| comparable >= limit);
        if policy.on_violation == Some(OnViolation::EndSession) || limit_reached {
            tracing::warn!("[realtime guardrail] ending session after violation {count}");
            close_backend(&mut *runner).await?;
        }
        let logged: String = transcript.chars().take(LOGGED_TRANSCRIPT_CHARS).collect();
        tracing::warn!("[realtime guardrail] BLOCKED transcript (violation {count}): {logged:?}");
        Ok(())
    }
    .boxed()
}

fn close_backend<'call, Config, CallbackError>(
    runner: &'call mut RealtimeGuardrails<'_, Config, CallbackError>,
) -> UnitFuture<'call, Config::Error, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async move {
        let backend = runner
            .state
            .backend
            .as_mut()
            .ok_or(RealtimeGuardrailsError::BackendMissing)?;
        backend
            .close()
            .await
            .map_err(RealtimeGuardrailsError::Close)
    }
    .boxed()
}
