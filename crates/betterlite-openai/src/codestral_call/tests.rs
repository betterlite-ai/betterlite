#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, TimeZone as _, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};

use crate::codestral_call::error::CodestralCallError;
use crate::codestral_call::log::{CallLogger, PreCall, StreamPostCall};
use crate::codestral_call::plan::{CodestralRequest, Mode, Plan, Wrapper, prepare};
use crate::codestral_records::options::CodestralSettings;
use crate::codestral_records::reply::{PostCall, PostCallLogger};
use crate::openai_base_config::Headers;

mod plan;
mod transport;

#[derive(Clone)]
struct PreRecord {
    input: String,
    api_key: Option<String>,
    body: String,
    headers: String,
    api_base: String,
    acompletion: bool,
}

#[derive(Clone)]
enum Event {
    Pre(PreRecord),
    Post,
    StreamPost(String, String),
}

#[derive(Clone, Default)]
struct Recorder {
    events: Arc<Mutex<Vec<Event>>>,
}

impl Recorder {
    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    fn phases(&self) -> Vec<&'static str> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|event| match event {
                Event::Pre(_) => "pre_call",
                Event::Post => "post_call",
                Event::StreamPost(_, _) => "stream_post_call",
            })
            .collect()
    }
}

impl PostCallLogger for Recorder {
    fn post_call(&mut self, _call: PostCall<'_>) {
        self.events.lock().unwrap().push(Event::Post);
    }
}

impl CallLogger for Recorder {
    fn pre_call(&mut self, call: PreCall<'_>) {
        self.events.lock().unwrap().push(Event::Pre(PreRecord {
            input: call.input.to_owned(),
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            body: serde_json::to_string(call.body).unwrap(),
            headers: serde_json::to_string(call.headers).unwrap(),
            api_base: call.api_base.to_owned(),
            acompletion: call.acompletion,
        }));
    }

    fn stream_post_call(&mut self, call: StreamPostCall<'_>) {
        self.events.lock().unwrap().push(Event::StreamPost(
            call.api_key.to_owned(),
            call.request_data.to_owned(),
        ));
    }
}

struct Fixture {
    key: Option<SecretString>,
    headers: Headers,
    model: String,
    messages: Vec<Message>,
    api_base: Option<String>,
    custom_prompts: BTreeMap<String, CustomPromptTemplate>,
    optional_params: Map<String, Value>,
    settings: CodestralSettings,
    mode: Mode,
    timeout: Duration,
}

fn base() -> Fixture {
    Fixture {
        key: Some(SecretString::from("sk-codestral")),
        headers: Headers::new(),
        model: "codestral-latest".to_owned(),
        messages: Vec::new(),
        api_base: None,
        custom_prompts: BTreeMap::new(),
        optional_params: Map::new(),
        settings: CodestralSettings::default(),
        mode: Mode::Sync,
        timeout: Duration::from_secs(12),
    }
}

impl Fixture {
    fn request(&self) -> CodestralRequest<'_> {
        CodestralRequest {
            api_key: self.key.as_ref(),
            headers: &self.headers,
            model: &self.model,
            messages: &self.messages,
            api_base: self.api_base.as_deref(),
            custom_prompts: &self.custom_prompts,
            optional_params: &self.optional_params,
            settings: &self.settings,
            mode: self.mode,
            timeout: self.timeout,
        }
    }
}

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).unwrap()
}

fn optional(text: &str) -> Map<String, Value> {
    serde_json::from_str(text).unwrap()
}

fn settings(text: &str) -> CodestralSettings {
    serde_json::from_str(text).unwrap()
}

async fn prepare_for(request: &CodestralRequest<'_>) -> Result<Plan, CodestralCallError> {
    prepare(request, &factory(), now(), &mut Recorder::default()).await
}

async fn prepare_error(request: &CodestralRequest<'_>) -> CodestralCallError {
    match prepare_for(request).await {
        Ok(_plan) => panic!("the plan must be rejected"),
        Err(error) => error,
    }
}
