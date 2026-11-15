use std::collections::{BTreeMap, BTreeSet};

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::groq_chat::error::GroqChatError;
use betterlite_openai::groq_stt::GroqSttError;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

mod capabilities;
mod chunks;
mod costs;
mod credentials;
mod messages;
mod options;
mod replies;
mod stt;
mod supported;

use capabilities::Answers;

pub(crate) struct Op;

// every section shares one call shape, and deny_unknown_fields still rejects a key no section reads
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Call {
    model: Option<String>,
    messages: Vec<ChatMessage>,
    environment: BTreeMap<String, String>,
    non_default: Object,
    optional_params: Object,
    drop_params: Option<bool>,
    global_drop_params: Option<bool>,
    reply: Option<Value>,
    json_mode: Option<bool>,
    chunk: Option<Value>,
    api_base: Option<String>,
    api_key: Option<String>,
    environment_base: Option<String>,
    environment_key: Option<String>,
    web_search_requests: Option<u64>,
    browser_open_requests: Option<u64>,
    medium_rate: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    capabilities: Answers,
    credentials: Vec<Call>,
    supported: Vec<Call>,
    messages: Vec<Call>,
    options: Vec<Call>,
    replies: Vec<Call>,
    chunks: Vec<Call>,
    stt: Vec<Call>,
    costs: Vec<Call>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = &input.capabilities;
        Ok(json!({
            "credentials": input.credentials.iter().map(credentials::credential).collect::<Vec<_>>(),
            "supported": projected(&input.supported, |call| supported::supported_one(answers, call)),
            "messages": projected(&input.messages, messages::message_one),
            "options": projected(&input.options, |call| options::options_one(answers, call)),
            "replies": projected(&input.replies, replies::reply_one),
            "chunks": chunks::chunk_values(&input.chunks),
            "stt": projected(&input.stt, stt::stt_one),
            "costs": costs::cost_values(&input.costs)?,
        }))
    }
}

// the Python adapter reports an exception's status as null when it has none, so both sides project null
trait Failure {
    fn status(&self) -> Value;
}

impl Failure for GroqChatError {
    fn status(&self) -> Value {
        match self {
            Self::Capability(_) => json!(null),
            other => json!(other.status_code()),
        }
    }
}

impl Failure for GroqSttError {
    fn status(&self) -> Value {
        json!(self.status_code())
    }
}

fn projected<Step, Error, Section>(calls: &[Step], section: Section) -> Vec<Value>
where
    Section: Fn(&Step) -> Result<Value, Error>,
    Error: Failure,
{
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let value = match section(call) {
            Ok(value) => value,
            Err(error) => json!({"error": {"status": error.status()}}),
        };
        values.push(value);
    }
    values
}

fn helper_error(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from(class),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

const fn empty_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

mod tests;
