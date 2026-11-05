#![cfg(test)]

use std::collections::VecDeque;
use std::future::ready;
use std::sync::{Arc, Mutex};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::Value;

use crate::assistants::client::ClientOptions;
use crate::assistants::operations::AssistantsApi;
use crate::assistants::transport::AssistantsTransport;

pub(super) const KEY: &str = "sk-fixture";
pub(super) const BASE: &str = "https://example.test/v1";
pub(super) const ASSISTANT: &str = r#"{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[{"type":"code_interpreter"}],"tool_resources":{"code_interpreter":{"file_ids":["file-1"]}},"response_format":"auto","temperature":0.5}"#;
pub(super) const PAGE: &str =
    r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
pub(super) const DELETED: &str = r#"{"id":"asst_1","deleted":true,"object":"assistant.deleted"}"#;

// What the fake transport saw of one request, with the secret headers read back as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) authorization: Option<String>,
    pub(super) organization: Option<String>,
    pub(super) beta: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) body: Option<Value>,
}

#[derive(Default)]
pub(super) struct Recorder {
    seen: Mutex<Vec<Seen>>,
    debug: Mutex<Vec<String>>,
    reply: Mutex<VecDeque<(u16, String)>>,
}

impl Recorder {
    pub(super) fn queue(&self, status: u16, body: &str) {
        self.reply
            .lock()
            .unwrap()
            .push_back((status, body.to_owned()));
    }

    pub(super) fn debug_texts(&self) -> Vec<String> {
        self.debug.lock().unwrap().clone()
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
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
        self.debug.lock().unwrap().push(format!("{request:?}"));
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = match &request.body {
            RequestBody::Json(bytes) => Some(serde_json::from_slice(bytes).unwrap()),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
        };
        // The transport adds this header itself for a JSON body.
        let content_type = header_of(&request.headers, "content-type")
            .or_else(|| body.is_some().then(|| String::from("application/json")));
        self.seen.lock().unwrap().push(Seen {
            method: request.method.as_str().to_owned(),
            url: url.as_str().to_owned(),
            authorization: header_of(&request.headers, "authorization"),
            organization: header_of(&request.headers, "openai-organization"),
            beta: header_of(&request.headers, "openai-beta"),
            content_type,
            body,
        });
        let (status, text) = self.reply.lock().unwrap().pop_front().unwrap();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: text.into_bytes(),
        }))
    }
}

pub(super) fn options(key: &str) -> ClientOptions {
    ClientOptions {
        api_key: Some(SecretString::from(key)),
        api_base: Some(String::from(BASE)),
        timeout: None,
        max_retries: None,
        organization: Some(String::from("org-fixture")),
    }
}

pub(super) fn api_with(status: u16, body: &str) -> (AssistantsApi<Recorder>, Arc<Recorder>) {
    let transport = Arc::new(Recorder::default());
    transport.queue(status, body);
    let api = AssistantsApi {
        transport: Arc::clone(&transport),
    };
    (api, transport)
}

pub(super) fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

// The request every case expects: bearer key, organization and the assistants beta header.
pub(super) fn wire(method: &str, url: &str, body: Option<&str>) -> Seen {
    Seen {
        method: method.to_owned(),
        url: url.to_owned(),
        authorization: Some(format!("Bearer {KEY}")),
        organization: Some(String::from("org-fixture")),
        beta: Some(String::from("assistants=v2")),
        content_type: (method != "GET").then(|| String::from("application/json")),
        body: body.map(parse),
    }
}
