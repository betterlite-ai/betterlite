#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

pub(crate) const NO_INPUTS: GuardrailApiInputs = GuardrailApiInputs {
    texts: None,
    images: None,
    tools: None,
    tool_calls: None,
    structured_messages: None,
    model: None,
    stream_holdback_chars: None,
};

#[derive(Clone, Debug)]
pub(crate) struct Recording {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
}

pub(crate) struct ScriptedGuardrail {
    answer: GuardrailApiInputs,
    covers_full_request: bool,
    recorded: Mutex<Option<Recording>>,
}

impl ScriptedGuardrail {
    pub(crate) fn new(answer: GuardrailApiInputs) -> Self {
        Self {
            answer,
            covers_full_request: false,
            recorded: Mutex::new(None),
        }
    }

    pub(crate) fn covering_full_request(mut self) -> Self {
        self.covers_full_request = true;
        self
    }

    pub(crate) fn ran(&self) -> bool {
        self.recorded
            .lock()
            .expect("the recording lock is not poisoned")
            .is_some()
    }

    pub(crate) fn recording(&self) -> Recording {
        self.recorded
            .lock()
            .expect("the recording lock is not poisoned")
            .clone()
            .expect("the guardrail recorded its call")
    }
}

impl GuardrailContract for ScriptedGuardrail {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let recording = Recording {
            inputs,
            input_type,
            request_data: request_data.clone(),
        };
        *self
            .recorded
            .lock()
            .expect("the recording lock is not poisoned") = Some(recording);
        Ok(self.answer.clone())
    }

    fn messages_cover_full_request(&self) -> bool {
        self.covers_full_request
    }
}
