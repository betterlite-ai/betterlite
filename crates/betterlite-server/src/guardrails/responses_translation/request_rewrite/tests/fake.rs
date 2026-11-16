#![cfg(test)]

use std::future::Future;
use std::sync::Mutex;

use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use serde_json::Value;

#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: Object,
}

// Answers with the scripted inputs, or echoes what it was sent when nothing is scripted
pub(crate) struct FakeGuardrail {
    reply: Option<GuardrailApiInputs>,
    failure: Option<GuardrailFailure>,
    covers_full: bool,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    pub(crate) fn echoing() -> Self {
        Self {
            reply: None,
            failure: None,
            covers_full: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn replying(reply: Value) -> Self {
        Self {
            reply: Some(serde_json::from_value(reply).unwrap()),
            failure: None,
            covers_full: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing(failure: GuardrailFailure) -> Self {
        Self {
            reply: None,
            failure: Some(failure),
            covers_full: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn covering_full_requests(mut self) -> Self {
        self.covers_full = true;
        self
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().unwrap().clone()
    }
}

impl GuardrailContract for FakeGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut Object,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.calls.lock().unwrap().push(Recorded {
            inputs: inputs.clone(),
            input_type,
            request_data: request_data.clone(),
        });
        let outcome = match (&self.failure, &self.reply) {
            (Some(failure), _) => Err(failure.clone()),
            (None, Some(reply)) => Ok(reply.clone()),
            (None, None) => Ok(inputs),
        };
        async move { outcome }
    }

    fn messages_cover_full_request(&self) -> bool {
        self.covers_full
    }
}
