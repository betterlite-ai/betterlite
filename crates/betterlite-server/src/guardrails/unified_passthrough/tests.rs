#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_translation::TranslationCall;

use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;

pub(crate) fn map_body(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).expect("valid JSON fixture")
}

pub(crate) fn json_value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).expect("valid JSON fixture")
}

pub(crate) fn response_call<'call>(
    request_data: Option<&'call mut BTreeMap<String, GuardrailValue>>,
    user_api_key_dict: Option<&'call BTreeMap<String, GuardrailValue>>,
    logging_obj: Option<&'call mut GuardrailLogTarget>,
) -> TranslationCall<'call, GuardrailLogTarget> {
    TranslationCall {
        user_api_key_dict,
        request_data,
        logging_obj,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub(crate) texts: Vec<String>,
    pub(crate) model: Option<String>,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
    pub(crate) logged: bool,
}

enum Answer {
    Unchanged,
    Texts(Vec<String>),
    Fails(GuardrailFailure),
}

pub(crate) struct FakeGuardrail {
    answer: Answer,
    recorded: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    pub(crate) fn unchanged() -> Self {
        Self {
            answer: Answer::Unchanged,
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn rewriting(texts: &[&str]) -> Self {
        Self {
            answer: Answer::Texts(texts.iter().map(|text| (*text).to_owned()).collect()),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing(failure: GuardrailFailure) -> Self {
        Self {
            answer: Answer::Fails(failure),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
        self.recorded.lock().expect("not poisoned").clone()
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
        self.recorded.lock().expect("not poisoned").push(Recorded {
            texts: inputs.texts.clone().unwrap_or_default(),
            model: inputs.model.clone(),
            input_type,
            request_data: request_data.clone(),
            logged: logging_obj.is_some(),
        });
        let answer = match &self.answer {
            Answer::Unchanged => Ok(inputs),
            Answer::Texts(texts) => Ok(GuardrailApiInputs {
                texts: Some(texts.clone()),
                ..inputs
            }),
            Answer::Fails(failure) => Err(failure.clone()),
        };
        ready(answer)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("scripted hook failure")]
pub(crate) struct HookFailure;

#[derive(Clone, Debug, Default)]
pub(crate) struct Seen {
    pub(crate) data: Option<BTreeMap<String, GuardrailValue>>,
    pub(crate) key: Option<BTreeMap<String, GuardrailValue>>,
    pub(crate) response: Option<GuardrailValue>,
}

pub(crate) struct TestHook {
    replacement: Option<String>,
    seen: Mutex<Seen>,
}

impl TestHook {
    pub(crate) fn echoing() -> Self {
        Self {
            replacement: None,
            seen: Mutex::new(Seen::default()),
        }
    }

    pub(crate) fn replacing(text: &str) -> Self {
        Self {
            replacement: Some(text.to_owned()),
            seen: Mutex::new(Seen::default()),
        }
    }

    pub(crate) fn seen(&self) -> Seen {
        self.seen.lock().expect("not poisoned").clone()
    }
}

impl PostCallSuccessHook for TestHook {
    type Error = HookFailure;

    fn post_call_success_hook(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        user_api_key_dict: &BTreeMap<String, GuardrailValue>,
        response: GuardrailValue,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Error>> + Send {
        let mut seen = self.seen.lock().expect("not poisoned");
        seen.data = Some(data.clone());
        seen.key = Some(user_api_key_dict.clone());
        seen.response = Some(response.clone());
        drop(seen);
        let replacement = self.replacement.clone();
        ready(replacement.map_or(Ok(response), |text| Ok(answer_with_text(&text))))
    }
}

fn answer_with_text(text: &str) -> GuardrailValue {
    let block = GuardrailValue::Object(BTreeMap::from([(
        "text".to_owned(),
        GuardrailValue::Text(text.to_owned()),
    )]));
    let message = GuardrailValue::Object(BTreeMap::from([
        (
            "role".to_owned(),
            GuardrailValue::Text("assistant".to_owned()),
        ),
        ("content".to_owned(), GuardrailValue::Array(vec![block])),
    ]));
    let output = GuardrailValue::Object(BTreeMap::from([("message".to_owned(), message)]));
    GuardrailValue::Object(BTreeMap::from([("output".to_owned(), output)]))
}
