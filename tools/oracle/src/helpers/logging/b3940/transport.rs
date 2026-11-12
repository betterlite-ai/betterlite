use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Mutex, PoisonError};

use betterlite_logging::datadog::cost_management::datadog_cost_management::transport::{
    PutRequest, PutResponse, Transport,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Deserialize, Default)]
#[serde(default)]
pub(super) struct Reply {
    pub(super) status: u16,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ScriptedError {
    #[error("the scripted transport has no reply left")]
    NoReply,
    #[error("the cost upload body is not JSON")]
    Body(#[source] serde_json::Error),
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn header(request: &PutRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

impl Transport for ScriptedTransport {
    type Error = ScriptedError;

    fn put(
        &self,
        request: PutRequest,
    ) -> impl Future<Output = Result<PutResponse, Self::Error>> + Send {
        let url = request.url.to_string();
        let content_type = header(&request, "content-type");
        let credential = header(&request, "dd-api-key");
        let application = header(&request, "dd-application-key");
        let raw = request.body;
        async move {
            let body = serde_json::from_slice::<Value>(&raw).map_err(ScriptedError::Body)?;
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(json!({
                    "url": url,
                    "method": "PUT",
                    "content_type": content_type,
                    "api_key": credential,
                    "app_key": application,
                    "body": body,
                }));
            let reply = self
                .replies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .ok_or(ScriptedError::NoReply)?;
            Ok(PutResponse {
                status: reply.status,
            })
        }
    }
}
