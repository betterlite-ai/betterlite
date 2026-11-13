use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::{Criterion, CriterionWeight, JudgeParams};
use betterlite_prompts::arguments::json::{Json as ContentJson, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use serde::Serialize;

use crate::judge::apply::{
    JudgeInputs, LLMAsAJudgeGuardrail, OnFailure, RequestData, RequestMetadata,
};
use crate::judge::call::{
    JudgeCompletion, JudgeCompletionRequest, JudgeCompletionResponse, JudgeReply,
};
use crate::judge::config::{GuardrailEntry, LitellmParamsSource};
use crate::judge::hooks::{EventHookConfig, GuardrailEventHook};
use crate::judge::initialize::CallbackRegistry;
use crate::judge::record::JudgeGuardrailRecord;

pub(crate) fn params_with_model(model: Option<String>) -> JudgeParams {
    JudgeParams {
        judge_model: model,
        criteria: None,
        overall_threshold: None,
        on_failure: None,
        mode: None,
        default_on: None,
    }
}

pub(crate) fn criterion(name: &str, weight: CriterionWeight, description: &str) -> Criterion {
    Criterion {
        name: name.to_owned(),
        description: description.to_owned(),
        weight,
    }
}

pub(crate) fn criteria_100() -> Vec<Criterion> {
    vec![
        criterion("Accuracy", CriterionWeight::Integer(60), "Is it accurate?"),
        criterion("Safety", CriterionWeight::Integer(40), "Is it safe?"),
    ]
}

pub(crate) fn message(role: &str, content: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn part(kind: &str, name: &str, value: &str) -> ContentPart {
    let mut fields = Object::new();
    fields.insert(name.to_owned(), ContentJson::Text(value.to_owned()));
    ContentPart {
        kind: kind.to_owned(),
        fields,
    }
}

pub(crate) fn text_part(text: &str) -> ContentPart {
    part("text", "text", text)
}

pub(crate) fn image_part() -> ContentPart {
    part("image_url", "image_url", "data:image/png;base64,AAAA")
}

pub(crate) fn multipart(role: &str, parts: Vec<ContentPart>) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

pub(crate) fn texts(values: &[&str]) -> JudgeInputs {
    JudgeInputs {
        texts: Some(values.iter().map(|value| (*value).to_owned()).collect()),
        structured_messages: None,
        rest: BTreeMap::default(),
    }
}

pub(crate) fn texts_and_messages(values: &[&str], messages: Vec<Message>) -> JudgeInputs {
    JudgeInputs {
        texts: Some(values.iter().map(|value| (*value).to_owned()).collect()),
        structured_messages: Some(messages),
        rest: BTreeMap::default(),
    }
}

pub(crate) fn request_data(messages: Vec<Message>) -> RequestData {
    RequestData {
        messages: Some(messages),
        metadata: RequestMetadata::default(),
        rest: BTreeMap::default(),
    }
}

#[derive(Clone)]
pub(crate) enum Outcome {
    Reply(Option<String>),
    NoChoices,
    Failure,
}

pub(crate) struct RecordingCompletion {
    outcome: Outcome,
    requests: Mutex<Vec<JudgeCompletionRequest>>,
}

impl RecordingCompletion {
    pub(crate) fn new(outcome: Outcome) -> Arc<Self> {
        Arc::new(Self {
            outcome,
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn only_request(&self) -> JudgeCompletionRequest {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(requests.len(), 1, "the judge must be called exactly once");
        requests.remove(0)
    }

    pub(crate) fn call_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl JudgeCompletion for RecordingCompletion {
    type Error = io::Error;

    async fn complete(
        &self,
        request: &JudgeCompletionRequest,
    ) -> Result<JudgeCompletionResponse, Self::Error> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        match &self.outcome {
            Outcome::Reply(content) => Ok(JudgeCompletionResponse {
                choices: vec![JudgeReply {
                    content: content.clone(),
                }],
            }),
            Outcome::NoChoices => Ok(JudgeCompletionResponse {
                choices: Vec::new(),
            }),
            Outcome::Failure => Err(io::Error::other("judge endpoint down")),
        }
    }
}

#[derive(Default)]
pub(crate) struct RecordingRegistry {
    pub(crate) registered: Vec<Arc<LLMAsAJudgeGuardrail<io::Error>>>,
}

impl CallbackRegistry<io::Error> for RecordingRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<LLMAsAJudgeGuardrail<io::Error>>) {
        self.registered.push(guardrail);
    }
}

pub(crate) fn guardrail_with(
    completion: Arc<RecordingCompletion>,
) -> LLMAsAJudgeGuardrail<io::Error> {
    LLMAsAJudgeGuardrail {
        guardrail_name: "test_judge".to_owned(),
        judge_model: "gpt-4o-mini".to_owned(),
        criteria: criteria_100(),
        overall_threshold: 80.0,
        on_failure: OnFailure::Block,
        event_hook: EventHookConfig::Single(GuardrailEventHook::PostCall),
        default_on: false,
        completion,
        now: fixed_now,
    }
}

pub(crate) fn only_record(data: &RequestData) -> &JudgeGuardrailRecord {
    let records = &data.metadata.guardrail_records;
    assert_eq!(records.len(), 1, "a judge run leaves exactly one record");
    &records[0]
}

pub(crate) fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap_or(UNIX_EPOCH)
}

#[derive(Serialize)]
struct ReplyEntry {
    criterion_name: &'static str,
    score: f64,
    reasoning: &'static str,
    passed: bool,
    weight: u32,
}

#[derive(Serialize)]
struct ReplyPayload {
    verdicts: Vec<ReplyEntry>,
    overall_score: f64,
}

pub(crate) fn reply_with_score(score: f64) -> String {
    let entry = ReplyEntry {
        criterion_name: "Accuracy",
        score,
        reasoning: "ok",
        passed: true,
        weight: 60,
    };
    serde_json::to_string(&ReplyPayload {
        verdicts: vec![entry],
        overall_score: score,
    })
    .expect("reply serializes")
}

fn weight(value: i64) -> GuardrailValue {
    GuardrailValue::SignedInteger(value)
}

pub(crate) fn criterion_value(name: &str, value: i64, description: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        ("name".to_owned(), GuardrailValue::Text(name.to_owned())),
        (
            "description".to_owned(),
            GuardrailValue::Text(description.to_owned()),
        ),
        ("weight".to_owned(), weight(value)),
    ]))
}

pub(crate) fn valid_mapping() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        (
            "judge_model".to_owned(),
            GuardrailValue::Text("gpt-4o-mini".to_owned()),
        ),
        (
            "criteria".to_owned(),
            GuardrailValue::Array(vec![
                criterion_value("Accuracy", 60, "Is it accurate?"),
                criterion_value("Safety", 40, "Is it safe?"),
            ]),
        ),
        ("overall_threshold".to_owned(), weight(80)),
        (
            "on_failure".to_owned(),
            GuardrailValue::Text("block".to_owned()),
        ),
    ])
}

pub(crate) fn entry_from(mapping: &BTreeMap<String, GuardrailValue>) -> GuardrailEntry<'_> {
    GuardrailEntry {
        guardrail_name: Some("judge"),
        litellm_params: Some(LitellmParamsSource::Mapping(mapping)),
    }
}
