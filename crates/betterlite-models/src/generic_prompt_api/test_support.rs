#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use crate::decisions::Json;
use crate::generic_prompt_api::manager::{GenericPromptManager, GenericPromptSettings};
use crate::generic_prompt_api::request::{PromptApiRequest, PromptApiResponse, PromptApiTransport};
use crate::prompt_contracts::{PromptIgnoreFlags, PromptParams};

#[derive(Debug, Error)]
#[error("connection refused")]
pub(crate) struct FakeFailure;

#[derive(Clone, Debug)]
pub(crate) enum FakeOutcome {
    Answer { status: u16, body: String },
    Failure,
}

#[derive(Clone, Debug)]
pub(crate) struct FakeTransport {
    pub(crate) outcome: FakeOutcome,
    pub(crate) requests: Arc<Mutex<Vec<PromptApiRequest>>>,
}

impl FakeTransport {
    fn answer(&self, request: &PromptApiRequest) -> Result<PromptApiResponse, FakeFailure> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        match &self.outcome {
            FakeOutcome::Answer { status, body } => Ok(PromptApiResponse {
                status: *status,
                body: body.clone().into_bytes(),
            }),
            FakeOutcome::Failure => Err(FakeFailure),
        }
    }
}

impl PromptApiTransport for FakeTransport {
    type Error = FakeFailure;

    fn get(&self, request: &PromptApiRequest) -> Result<PromptApiResponse, FakeFailure> {
        self.answer(request)
    }

    fn get_async(
        &self,
        request: &PromptApiRequest,
    ) -> impl Future<Output = Result<PromptApiResponse, FakeFailure>> + Send {
        ready(self.answer(request))
    }
}

pub(crate) const TEMPLATE_BODY: &str = r#"{
    "prompt_id": "ignored-by-parser",
    "prompt_template": [
        {"role": "system", "content": "Hello {name}, welcome to {{place}}. {name} again."},
        {"role": "user", "content": "Ask about {topic}"}
    ],
    "prompt_template_model": "gpt-4",
    "prompt_template_optional_params": {"temperature": 0.7}
}"#;

pub(crate) fn settings(api_key: Option<&str>) -> GenericPromptSettings {
    GenericPromptSettings {
        api_base: "https://prompts.example.com/".to_owned(),
        api_key: api_key.map(SecretString::from),
        timeout: Duration::from_secs(30),
        prompt_id: None,
        provider_specific_query_params: IndexMap::new(),
        ignore_flags: PromptIgnoreFlags::default(),
    }
}

pub(crate) fn manager_with(
    outcome: FakeOutcome,
    settings: GenericPromptSettings,
) -> (
    GenericPromptManager<FakeTransport>,
    Arc<Mutex<Vec<PromptApiRequest>>>,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = FakeTransport {
        outcome,
        requests: Arc::clone(&requests),
    };
    (GenericPromptManager::new(settings, transport), requests)
}

pub(crate) fn template_manager() -> (
    GenericPromptManager<FakeTransport>,
    Arc<Mutex<Vec<PromptApiRequest>>>,
) {
    let outcome = FakeOutcome::Answer {
        status: 200,
        body: TEMPLATE_BODY.to_owned(),
    };
    manager_with(outcome, settings(None))
}

pub(crate) fn variables(pairs: &[(&str, Json)]) -> PromptParams {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn system_content(messages: &[Json]) -> Option<&str> {
    let Json::Object(fields) = messages.first()? else {
        return None;
    };
    match fields.get("content")? {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
