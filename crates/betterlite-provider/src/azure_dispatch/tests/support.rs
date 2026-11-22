use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_azure::azure_chat_base_policy::AzureChatSettings;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{Headers, OpenAiSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Map, Value, json};

use crate::azure_dispatch::AzureResult;
use crate::azure_dispatch::call::{ChatCall, Composition, Credentials, EmbeddingCall, TextCall};
use crate::azure_dispatch::dispatch::{Answer, AnswerReply};
use crate::prepared::PreparedHttp;

pub(crate) const CHAT_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o-2024-08-06","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
pub(crate) const EMBEDDING_REPLY: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,0.5]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
pub(crate) const TEXT_REPLY: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"gpt-3.5-turbo-instruct","choices":[{"index":0,"text":"hello","finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

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

pub(crate) fn text(model: &str, optional: Object, credentials: Credentials) -> TextCall {
    TextCall {
        model: model.to_owned(),
        messages: messages(),
        optional,
        credentials,
        acompletion: false,
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

pub(crate) fn value(result: &AzureResult) -> Value {
    serde_json::to_value(result).expect("the dispatch result")
}

// The answer port a case supplies: one fixed reply, and every request it was handed.
pub(crate) struct Fixture {
    status: u16,
    body: Vec<u8>,
    sent: Mutex<Vec<Value>>,
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

    pub(crate) fn sent(&self) -> Vec<Value> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Answer for Fixture {
    fn answer(&self, request: &PreparedHttp) -> AnswerReply {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let headers: Map<String, Value> = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    Value::String(value.to_str().unwrap_or_default().to_owned()),
                )
            })
            .collect();
        let entry = json!({
            "method": request.method.as_str(),
            "url": request.url.as_str(),
            "headers": headers,
            "body": body,
        });
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
        AnswerReply {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }
    }
}
