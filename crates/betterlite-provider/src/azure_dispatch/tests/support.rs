use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_azure::azure_chat_base_policy::AzureChatSettings;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{Headers, OpenAiSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use http::HeaderMap;
use secrecy::SecretString;

use crate::azure_dispatch::call::{ChatCall, Composition, Credentials, EmbeddingCall, TextCall};
use crate::azure_dispatch::fixtures::execution::recorded;
use crate::azure_dispatch::fixtures::types::{Answer, AnswerReply, AzureResult, RecordedRequest};
use crate::prepared::PreparedHttp;

pub(crate) const CHAT_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o-2024-08-06","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
pub(crate) const EMBEDDING_REPLY: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,0.5]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
pub(crate) const TEXT_REPLY: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"gpt-3.5-turbo-instruct","choices":[{"index":0,"text":"hello","finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
pub(crate) const AZURE_BASE: &str = "https://resource.openai.azure.com";

const USER: &str = r#"[{"role":"user","content":"hi"}]"#;

pub(crate) fn composition(environment: &[(&str, &str)]) -> Composition {
    let mut values = BTreeMap::new();
    for (name, value) in environment {
        values.insert((*name).to_owned(), (*value).to_owned());
    }
    Composition::from_environment(values)
}

pub(crate) fn azure_defaults(body: &str) -> AzureChatSettings {
    serde_json::from_str(body).expect("Azure chat defaults")
}

pub(crate) fn o_series_defaults(body: &str) -> OpenAiSettings {
    serde_json::from_str(body).expect("o-series defaults")
}

pub(crate) fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn json(body: &str) -> Json {
    serde_json::from_str(body).expect("JSON")
}

pub(crate) fn object(body: &str) -> Object {
    serde_json::from_str(body).expect("an options object")
}

pub(crate) fn messages() -> Vec<Message> {
    serde_json::from_str(USER).expect("the user message")
}

// The empty credential map, so a case names only the values it exercises.
pub(crate) const fn credentials() -> Credentials {
    Credentials {
        api_base: None,
        api_version: None,
        api_key: None,
        headers: None,
        extra_headers: None,
        max_retries: None,
        extra_body: None,
        azure_ad_token: None,
        base_model: None,
        timeout: None,
    }
}

pub(crate) fn base_credentials() -> Credentials {
    Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(key("call-key")),
        ..credentials()
    }
}

pub(crate) fn base_only() -> Credentials {
    Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..credentials()
    }
}

pub(crate) fn extra_credentials(body: &str) -> Credentials {
    Credentials {
        extra_body: Some(json(body)),
        ..base_credentials()
    }
}

pub(crate) fn text_call(optional: Object, credentials: Credentials, acompletion: bool) -> TextCall {
    TextCall {
        model: String::from("gpt-3.5-turbo-instruct"),
        messages: messages(),
        optional,
        credentials,
        acompletion,
    }
}

pub(crate) fn chat(model: &str, optional: Object, credentials: Credentials) -> ChatCall {
    ChatCall {
        model: model.to_owned(),
        messages: messages(),
        optional,
        credentials,
        stream: false,
        supplied: None,
    }
}

pub(crate) fn embedding(model: &str, body: &str, credentials: Credentials) -> EmbeddingCall {
    let request: EmbeddingRequest = serde_json::from_str(body).expect("an embedding request");
    EmbeddingCall {
        model: model.to_owned(),
        request,
        credentials,
    }
}

pub(crate) fn at<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    let fields = value.as_object()?;
    fields.get(name)
}

pub(crate) fn text_at<'value>(value: &'value Json, name: &str) -> Option<&'value str> {
    at(value, name).and_then(Json::as_str)
}

pub(crate) fn number_at(value: &Json, name: &str) -> Option<u64> {
    match at(value, name) {
        Some(Json::Number(number)) => number.as_u64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(crate) fn item(value: &Json, index: usize) -> Option<&Json> {
    match value {
        Json::List(items) => items.get(index),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

pub(crate) fn body_at<'request>(
    request: &'request RecordedRequest,
    name: &str,
) -> Option<&'request Json> {
    at(&request.body, name)
}

pub(crate) fn body_text<'request>(
    request: &'request RecordedRequest,
    name: &str,
) -> Option<&'request str> {
    text_at(&request.body, name)
}

pub(crate) fn body_number(request: &RecordedRequest, name: &str) -> Option<i64> {
    let value = number_at(&request.body, name)?;
    i64::try_from(value).ok()
}

pub(crate) fn parse(body: &[u8]) -> Json {
    serde_json::from_slice(body).expect("a JSON body")
}

pub(crate) fn header<'request>(
    request: &'request RecordedRequest,
    name: &str,
) -> Option<&'request str> {
    request.headers.get(name).map(String::as_str)
}

pub(crate) fn rendered(result: &AzureResult) -> String {
    serde_json::to_string(result).expect("the dispatch result")
}

pub(crate) fn sent_text(request: &RecordedRequest) -> String {
    serde_json::to_string(request).expect("the recorded request")
}

pub(crate) fn reply_text<'result>(
    result: &'result AzureResult,
    name: &str,
) -> Option<&'result str> {
    text_at(&result.reply, name)
}

pub(crate) fn error_status(result: &AzureResult) -> Option<u64> {
    number_at(&result.error, "status")
}

// The answer port a case supplies: one fixed reply, and every request it was handed.
pub(crate) struct Fixture {
    status: u16,
    body: Vec<u8>,
    sent: Mutex<Vec<RecordedRequest>>,
}

impl Fixture {
    pub(crate) fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn ok(body: &str) -> Self {
        Self::new(200, body)
    }

    pub(crate) fn sent(&self) -> Vec<RecordedRequest> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// A body the dispatch wrote is always JSON; a malformed one is recorded as null so the request still shows.
fn entry(request: &PreparedHttp) -> RecordedRequest {
    recorded(request).unwrap_or_else(|_error| RecordedRequest {
        method: request.method.as_str().to_owned(),
        url: request.url.to_string(),
        headers: BTreeMap::new(),
        body: Json::Null,
    })
}

impl Answer for Fixture {
    fn answer(&self, request: &PreparedHttp) -> AnswerReply {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry(request));
        AnswerReply {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }
    }
}
