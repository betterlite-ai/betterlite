use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use betterlite_core::api::realtime_guardrails::callback::{
    GuardrailApplication, GuardrailFailure, GuardrailInput, GuardrailTexts, GuardrailViolation,
    RealtimeGuardrail, RealtimeGuardrailPolicy,
};
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;

use super::transport::Locked as _;

pub(crate) struct ScriptedGuardrail {
    hooks: Vec<GuardrailEventHooks>,
    block_on: Option<String>,
    policy: RealtimeGuardrailPolicy,
    applied: Arc<Mutex<Vec<Vec<String>>>>,
}

impl ScriptedGuardrail {
    pub(crate) fn new(
        hooks: Vec<GuardrailEventHooks>,
        block_on: Option<String>,
        applied: Arc<Mutex<Vec<Vec<String>>>>,
    ) -> Self {
        Self {
            hooks,
            block_on,
            policy: RealtimeGuardrailPolicy::default(),
            applied,
        }
    }
}

impl RealtimeGuardrailCallback for ScriptedGuardrail {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        true
    }

    fn should_run_guardrail(
        &self,
        _data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        Ok(self.hooks.contains(&event))
    }
}

impl RealtimeGuardrail for ScriptedGuardrail {
    fn use_native_lifecycle_hooks(&self) -> bool {
        false
    }

    fn policy(&self) -> &RealtimeGuardrailPolicy {
        &self.policy
    }

    fn apply_guardrail(&self, input: GuardrailInput) -> GuardrailApplication<'_, Infallible> {
        self.applied.locked().push(input.texts.clone());
        let blocked = input
            .texts
            .first()
            .is_some_and(|text| self.block_on.as_deref() == Some(text.as_str()));
        Box::pin(async move {
            if blocked {
                return Err(GuardrailFailure::Blocked(GuardrailViolation {
                    detail: Json::Null,
                    message: String::from("blocked"),
                }));
            }
            Ok(GuardrailTexts::default())
        })
    }
}
