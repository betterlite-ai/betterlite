use super::input::Reply;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

pub(super) const THREAD: &str =
    r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"core"}}"#;
pub(super) const MESSAGE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"completed"}"#;
pub(super) const PAGE: &str = r#"{"object":"list","data":[{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"completed"}],"first_id":"msg_1","last_id":"msg_1","has_more":false}"#;

pub(super) struct Wire {
    pub(super) seen: Mutex<Vec<Value>>,
    replies: Mutex<VecDeque<(u16, String)>>,
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

impl AssistantsTransport for Wire {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let body: Option<Value> = match &request.body {
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
            RequestBody::Json(bytes) => serde_json::from_slice(bytes).ok(),
        };
        let content_type = header(&request.headers, "content-type")
            .or_else(|| body.is_some().then(|| String::from("application/json")));
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "method": request.method.as_str(),
                "url": url.as_str(),
                "authorization": header(&request.headers, "authorization"),
                "openai-organization": header(&request.headers, "openai-organization"),
                "openai-beta": header(&request.headers, "openai-beta"),
                "content-type": content_type,
                "body": body,
            }));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| (200, String::from("{}")));
        ready(Ok(UpstreamResponse {
            status: reply.0,
            headers: HeaderMap::new(),
            body: reply.1.into_bytes(),
        }))
    }
}

pub(super) fn replying(given: Option<&Reply>, canned: &str) -> Arc<Wire> {
    let reply = given.map_or_else(
        || vec![(200, String::from(canned))],
        |reply| vec![(reply.status, reply.body.clone())],
    );
    Arc::new(Wire {
        seen: Mutex::new(Vec::new()),
        replies: Mutex::new(VecDeque::from(reply)),
    })
}

pub(super) fn dispatch_wire(given: Option<&Reply>) -> Arc<Wire> {
    let mut replies = VecDeque::new();
    replies.push_back((200, String::from(THREAD)));
    replies.push_back((200, String::from(THREAD)));
    replies.push_back((200, String::from(MESSAGE)));
    replies.push_back(given.map_or_else(
        || (200, String::from(PAGE)),
        |reply| (reply.status, reply.body.clone()),
    ));
    Arc::new(Wire {
        seen: Mutex::new(Vec::new()),
        replies: Mutex::new(replies),
    })
}
