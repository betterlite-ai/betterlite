use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use tempfile::NamedTempFile;
use tracing::subscriber::{DefaultGuard, set_default};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
}

pub(crate) struct FakeGuardrail {
    reply: Option<Vec<String>>,
    drops_texts: bool,
    failure: Option<GuardrailFailure>,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    pub(crate) fn replying(texts: &[&str]) -> Self {
        Self {
            reply: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
            drops_texts: false,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn passing_through() -> Self {
        Self {
            reply: None,
            drops_texts: false,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn dropping_texts() -> Self {
        Self {
            reply: None,
            drops_texts: true,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing(failure: GuardrailFailure) -> Self {
        Self {
            reply: None,
            drops_texts: false,
            failure: Some(failure),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().unwrap().clone()
    }
}

impl GuardrailContract for FakeGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.calls.lock().unwrap().push(Recorded {
            inputs: inputs.clone(),
            input_type,
            request_data: request_data.clone(),
        });
        let reply = self.reply.clone();
        let drops_texts = self.drops_texts;
        let failure = self.failure.clone();
        async move {
            if let Some(blocked) = failure {
                return Err(blocked);
            }
            let mut outputs = inputs;
            match reply {
                Some(texts) => outputs.texts = Some(texts),
                None if drops_texts => outputs.texts = None,
                None => {}
            }
            Ok(outputs)
        }
    }
}

pub(crate) fn body(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn capture_logs(level: tracing::Level) -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (log, set_default(subscriber))
}

pub(crate) fn log_text(log: &NamedTempFile) -> String {
    fs::read_to_string(log.path()).unwrap()
}
