#![cfg(test)]

use std::collections::VecDeque;
use std::future::ready;
use std::sync::{Arc, Mutex};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::assistants::client::ClientOptions;
use crate::assistants::operations::AssistantsApi;
use crate::assistants::transport::AssistantsTransport;
use crate::threads::operations::{ClientSetup, ThreadsApi};

pub(super) const KEY: &str = "sk-fixture";
pub(super) const BASE: &str = "https://example.test/v1";
pub(super) const THREAD: &str = r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"core"},"tool_resources":{"code_interpreter":{"file_ids":["file-1"]}}}"#;
pub(super) const THREAD_PROJECTION: &str =
    r#"{"id":"thread_1","created_at":7,"metadata":{"team":"core"},"object":"thread"}"#;
pub(super) const THREAD_NULL: &str =
    r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":null}"#;
pub(super) const THREAD_NULL_PROJECTION: &str =
    r#"{"id":"thread_1","created_at":7,"metadata":null,"object":"thread"}"#;
pub(super) const THREAD_NUMERIC: &str =
    r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":{"n":1}}"#;
pub(super) const THREAD_NUMERIC_PROJECTION: &str =
    r#"{"id":"thread_1","created_at":7,"metadata":{"n":1},"object":"thread"}"#;
pub(super) const MESSAGE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"assistant_id":null,"attachments":null,"completed_at":null,"incomplete_at":null,"incomplete_details":null,"metadata":null,"run_id":null,"thread_id":"thread_1","role":"assistant","status":"completed","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"unknown_field":{"nested":[1,2]}}"#;
pub(super) const MESSAGE_NO_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"assistant_id":null,"attachments":null,"completed_at":null,"incomplete_at":null,"incomplete_details":null,"metadata":null,"run_id":null,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
pub(super) const MESSAGE_COMPLETED: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"assistant_id":null,"attachments":null,"completed_at":null,"incomplete_at":null,"incomplete_details":null,"metadata":null,"run_id":null,"thread_id":"thread_1","role":"assistant","status":"completed","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
pub(super) const MESSAGE_IN_PROGRESS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"assistant_id":null,"attachments":null,"completed_at":null,"incomplete_at":null,"incomplete_details":null,"metadata":null,"run_id":null,"thread_id":"thread_1","role":"assistant","status":"in_progress","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
pub(super) const MESSAGE_NULL_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"assistant_id":null,"attachments":null,"completed_at":null,"incomplete_at":null,"incomplete_details":null,"metadata":null,"run_id":null,"thread_id":"thread_1","role":"assistant","status":null,"content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
pub(super) const MESSAGE_EMPTY_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"assistant_id":null,"attachments":null,"completed_at":null,"incomplete_at":null,"incomplete_details":null,"metadata":null,"run_id":null,"thread_id":"thread_1","role":"assistant","status":"","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
pub(super) const MESSAGE_EXTRAS: &str = r#"{"id":"msg_9","object":"thread.message","created_at":9,"assistant_id":null,"attachments":[{"file_id":"file-1","tools":[{"type":"code_interpreter","extra_tool_key":7}]}],"completed_at":null,"incomplete_at":null,"incomplete_details":null,"metadata":null,"run_id":null,"thread_id":"thread_1","role":"assistant","status":"completed","content":[{"type":"text","text":{"value":"cited","annotations":[{"type":"file_citation","text":"s","start_index":0,"end_index":1,"file_citation":{"file_id":"file-1","quote":"q"},"extra_annotation":true}],"extra_text":1},"extra_block":"x"}]}"#;
pub(super) const PAGE: &str = r#"{"object":"list","data":[{"id":"msg_1","object":"thread.message","created_at":7,"assistant_id":null,"attachments":null,"completed_at":null,"incomplete_at":null,"incomplete_details":null,"metadata":null,"run_id":null,"thread_id":"thread_1","role":"assistant","status":"completed","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"unknown_field":{"nested":[1,2]}}],"first_id":"msg_1","last_id":"msg_1","has_more":false}"#;

#[derive(Default)]
pub(super) struct Recorder {
    requests: Mutex<Vec<Value>>,
    replies: Mutex<VecDeque<(u16, String)>>,
}

impl Recorder {
    pub(super) fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back((status, body.to_owned()));
    }

    pub(super) fn seen(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

fn header_of(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body: Option<Value> = match &request.body {
            RequestBody::Json(bytes) => serde_json::from_slice(bytes).ok(),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
        };
        let content_type = header_of(&request.headers, "content-type")
            .or_else(|| body.is_some().then(|| String::from("application/json")));
        let fields = vec![
            ("method", Value::from(request.method.as_str())),
            ("url", Value::from(url.as_str())),
            ("deadline", json!(request.deadline.as_secs_f64())),
            ("accept", json!(header_of(&request.headers, "accept"))),
            (
                "authorization",
                json!(header_of(&request.headers, "authorization")),
            ),
            (
                "organization",
                json!(header_of(&request.headers, "openai-organization")),
            ),
            ("beta", json!(header_of(&request.headers, "openai-beta"))),
            ("content-type", json!(content_type)),
            ("body", json!(body)),
        ];
        self.requests.lock().unwrap().push(object(fields));
        let (status, text) = self.replies.lock().unwrap().pop_front().unwrap();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: text.into_bytes(),
        }))
    }
}

pub(super) fn options(key: &str) -> ClientOptions {
    let api_base = Some(String::from(BASE));
    let organization = Some(String::from("org-fixture"));
    ClientOptions {
        api_key: Some(SecretString::from(key)),
        api_base,
        organization,
        timeout: None,
        max_retries: None,
    }
}

pub(super) fn setup(key: &str) -> ClientSetup<Recorder> {
    ClientSetup {
        options: options(key),
        supplied: None,
        async_requested: false,
    }
}

pub(super) fn async_setup(key: &str) -> ClientSetup<Recorder> {
    let mut setup = setup(key);
    setup.async_requested = true;
    setup
}

pub(super) fn api_with(status: u16, body: &str) -> (ThreadsApi<Recorder>, Arc<Recorder>) {
    let transport = Arc::new(Recorder::default());
    transport.queue(status, body);
    let api = ThreadsApi {
        assistants: AssistantsApi {
            transport: Arc::clone(&transport),
        },
    };
    (api, transport)
}

pub(super) fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

// Object keys go through this builder because serde_json's json! macro builds its map with Into::into, which clippy.toml disallows.
fn object(entries: Vec<(&str, Value)>) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in entries {
        let replaced = map.insert(key.to_owned(), value);
        debug_assert!(replaced.is_none(), "duplicate key {key}");
    }
    Value::Object(map)
}

pub(super) fn request(method: &str, url: &str, body: Option<&str>) -> Value {
    object(vec![
        ("method", Value::from(method)),
        ("url", Value::from(url)),
        ("deadline", json!(600.0)),
        ("accept", json!("application/json")),
        ("authorization", json!(format!("Bearer {KEY}"))),
        ("organization", json!("org-fixture")),
        ("beta", json!("assistants=v2")),
        (
            "content-type",
            json!((method != "GET").then_some("application/json")),
        ),
        ("body", json!(body.map(parse))),
    ])
}
