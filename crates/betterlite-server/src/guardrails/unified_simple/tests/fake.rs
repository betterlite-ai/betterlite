use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
    pub(crate) logged: bool,
}

pub(crate) type CallLog = Arc<Mutex<Vec<Recorded>>>;

pub(crate) struct FakeGuardrail {
    reply: Option<Vec<String>>,
    failure: Option<GuardrailFailure>,
    calls: CallLog,
}

impl FakeGuardrail {
    pub(crate) fn replying(texts: &[&str]) -> Self {
        Self {
            reply: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
            failure: None,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn failing(failure: GuardrailFailure) -> Self {
        Self {
            reply: None,
            failure: Some(failure),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn calls(&self) -> CallLog {
        Arc::clone(&self.calls)
    }
}

impl GuardrailContract for FakeGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.calls.lock().unwrap().push(Recorded {
            inputs,
            input_type,
            request_data: request_data.clone(),
            logged: logging_obj.is_some(),
        });
        let reply = self.reply.clone();
        let failure = self.failure.clone();
        async move {
            if let Some(blocked) = failure {
                return Err(blocked);
            }
            Ok(GuardrailApiInputs {
                texts: reply,
                images: None,
                tools: None,
                tool_calls: None,
                structured_messages: None,
                model: None,
                stream_holdback_chars: None,
            })
        }
    }
}

pub(crate) fn calls_of(log: &CallLog) -> Vec<Recorded> {
    log.lock().unwrap().clone()
}
