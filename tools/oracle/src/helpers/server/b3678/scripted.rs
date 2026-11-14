use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_server::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;

#[derive(Clone, Default)]
pub(super) struct Seen {
    pub(super) texts: Vec<String>,
    pub(super) model: Option<String>,
}

pub(super) struct ScriptedGuardrail {
    answer: Option<Vec<String>>,
    seen: Mutex<Seen>,
}

impl ScriptedGuardrail {
    pub(super) fn new(answer: Option<Vec<String>>) -> Self {
        Self {
            answer,
            seen: Mutex::new(Seen::default()),
        }
    }

    pub(super) fn seen(&self) -> Seen {
        match self.seen.lock() {
            Ok(seen) => seen.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn record(&self, inputs: &GuardrailApiInputs) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.texts
            .clone_from(&inputs.texts.clone().unwrap_or_default());
        seen.model.clone_from(&inputs.model);
    }
}

impl GuardrailContract for ScriptedGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        _input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.record(&inputs);
        let result = match &self.answer {
            Some(answer) => Ok(GuardrailApiInputs {
                texts: Some(answer.clone()),
                ..inputs
            }),
            None => Ok(inputs),
        };
        ready(result)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("scripted hook failure")]
pub(super) struct HookFailure;

pub(super) struct ScriptedHook {
    pub(super) replace: BTreeMap<String, String>,
}

impl PostCallSuccessHook for ScriptedHook {
    type Error = HookFailure;

    fn post_call_success_hook(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
        _user_api_key_dict: &BTreeMap<String, GuardrailValue>,
        response: GuardrailValue,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Error>> + Send {
        ready(Ok(replace_blocks(&response, &self.replace)))
    }
}

fn replace_blocks(response: &GuardrailValue, replace: &BTreeMap<String, String>) -> GuardrailValue {
    let mut replaced = response.clone();
    if let Some(blocks) = content_blocks_mut(&mut replaced) {
        for entry in blocks.iter_mut() {
            let GuardrailValue::Object(block) = entry else {
                continue;
            };
            let Some(GuardrailValue::Text(text)) = block.get_mut("text") else {
                continue;
            };
            if let Some(replacement) = replace.get(text.as_str()) {
                text.clone_from(replacement);
            }
        }
    }
    replaced
}

fn content_blocks_mut(response: &mut GuardrailValue) -> Option<&mut Vec<GuardrailValue>> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    let Some(GuardrailValue::Object(output)) = fields.get_mut("output") else {
        return None;
    };
    let Some(GuardrailValue::Object(message)) = output.get_mut("message") else {
        return None;
    };
    match message.get_mut("content") {
        Some(GuardrailValue::Array(blocks)) => Some(blocks),
        _ => None,
    }
}
