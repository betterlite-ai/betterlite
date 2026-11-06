use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::records::RunEvent;
use betterlite_openai::thread_runs::stream::RunEventHandler;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::input::Reply;
use super::invalid_input;

const NO_REPLY: &str = r#"{"error":{"message":"the fixture queues no reply"}}"#;

pub(super) struct Wire {
    seen: Mutex<Vec<Value>>,
    replies: Mutex<VecDeque<(u16, HeaderMap, Vec<u8>)>>,
}

impl Wire {
    pub(super) fn new(replies: &[Reply]) -> Result<Arc<Self>, Box<crate::helpers::HelperError>> {
        let mut queued = VecDeque::new();
        for reply in replies {
            let mut headers = HeaderMap::new();
            for (name, value) in &reply.headers {
                let header = HeaderName::try_from(name.as_str())
                    .map_err(|error| invalid_input(error.to_string()))?;
                let header_value = HeaderValue::try_from(value.as_str())
                    .map_err(|error| invalid_input(error.to_string()))?;
                headers.insert(header, header_value);
            }
            queued.push_back((reply.status, headers, reply.body.as_bytes().to_vec()));
        }
        Ok(Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            replies: Mutex::new(queued),
        }))
    }

    pub(super) fn requests(&self) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
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
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = match &request.body {
            RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Value::Null),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Value::Null
            }
        };
        let content_type = header(&request.headers, "content-type")
            .or_else(|| (!body.is_null()).then(|| String::from("application/json")));
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
            .unwrap_or_else(|| (500, HeaderMap::new(), NO_REPLY.as_bytes().to_vec()));
        ready(Ok(UpstreamResponse {
            status: reply.0,
            headers: reply.1,
            body: reply.2,
        }))
    }
}

pub(super) struct RecordingHandler {
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

impl RunEventHandler for RecordingHandler {
    fn on_event(&self, event: &RunEvent) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.name().to_owned());
    }

    fn on_end(&self) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(String::from("end"));
    }
}
