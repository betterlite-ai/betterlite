use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use betterlite_core::api::realtime_guardrails::callback::{
    GuardrailApplication, GuardrailFailure, GuardrailInput, GuardrailTexts, GuardrailViolation,
    RealtimeGuardrail, RealtimeGuardrailPolicy,
};
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;

use super::input::{GuardrailDocument, GuardrailKind, Outcome, Raised};
use super::transport::Locked as _;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct ScriptedError {
    pub(crate) message: String,
}

#[derive(Debug, Default)]
pub(crate) struct Records {
    pub(crate) checks: Vec<(GuardrailEventHooks, RequestData)>,
    pub(crate) applied: Vec<GuardrailInput>,
}

pub(crate) struct ScriptedGuardrail {
    document: GuardrailDocument,
    policy: RealtimeGuardrailPolicy,
    records: Arc<Mutex<Records>>,
}

impl ScriptedGuardrail {
    pub(crate) fn new(document: &GuardrailDocument, records: &Arc<Mutex<Records>>) -> Self {
        Self {
            policy: RealtimeGuardrailPolicy {
                violation_message: document.policy.violation_message.clone(),
                on_violation: document.policy.on_violation,
                end_session_after_n_fails: document.policy.end_session_after_n_fails,
            },
            document: document.clone(),
            records: Arc::clone(records),
        }
    }

    // A raised exception with a status_code or a ValueError blocks; every other exception is unexpected, detail or not
    fn failure(raised: &Option<Raised>) -> GuardrailFailure<ScriptedError> {
        let raised = raised.clone().unwrap_or_default();
        if raised.status_code.is_some() || raised.value_error {
            GuardrailFailure::Blocked(GuardrailViolation {
                detail: raised.detail.unwrap_or(Json::Null),
                message: raised.message,
            })
        } else {
            GuardrailFailure::Unexpected(ScriptedError {
                message: raised.message,
            })
        }
    }
}

impl RealtimeGuardrailCallback for ScriptedGuardrail {
    type Error = ScriptedError;

    fn is_custom_guardrail(&self) -> bool {
        matches!(
            self.document.kind,
            GuardrailKind::Custom | GuardrailKind::Native
        )
    }

    fn should_run_guardrail(
        &self,
        data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, ScriptedError> {
        self.records.locked().checks.push((event, data.clone()));
        // CustomGuardrail.should_run_guardrail with no request metadata: a default-on guardrail runs for a listed hook
        Ok(self.document.default_on && self.document.hooks.contains(&event))
    }
}

impl RealtimeGuardrail for ScriptedGuardrail {
    fn use_native_lifecycle_hooks(&self) -> bool {
        matches!(self.document.kind, GuardrailKind::Native)
    }

    fn policy(&self) -> &RealtimeGuardrailPolicy {
        &self.policy
    }

    fn apply_guardrail(&self, input: GuardrailInput) -> GuardrailApplication<'_, ScriptedError> {
        self.records.locked().applied.push(input);
        let outcome = self.document.outcome;
        let failure = Self::failure(&self.document.raised);
        Box::pin(async move {
            match outcome {
                Outcome::Clean => Ok(GuardrailTexts::default()),
                Outcome::Rewrite => Ok(GuardrailTexts(vec![String::from("the guardrail rewrite")])),
                Outcome::Raise => Err(failure),
            }
        })
    }
}
