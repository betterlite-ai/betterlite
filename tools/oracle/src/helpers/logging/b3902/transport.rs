use std::collections::VecDeque;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::langfuse_exporter::sleep::Sleep;
use betterlite_logging::langfuse_exporter::transport::{HttpRequest, HttpResponse, Transport};
use http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::journal::Journal;
use super::spans::span_json;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    transport_error: bool,
}

const fn default_status() -> u16 {
    200
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            status: 200,
            transport_error: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted connection failed")]
pub(crate) struct ScriptedError;

#[derive(Debug)]
pub(crate) struct ScriptedTransport {
    pub(crate) replies: Mutex<VecDeque<Reply>>,
    pub(crate) journal: Arc<Journal>,
}

impl Transport for ScriptedTransport {
    type Error = ScriptedError;

    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, ScriptedError>> + Send {
        let spans = <[u8; 8]>::try_from(request.body.as_ref())
            .ok()
            .map(u64::from_be_bytes)
            .and_then(|index| self.journal.encoded(index))
            .unwrap_or_default();
        self.journal.record_request(json!({
            "url": request.url.to_string(),
            "headers": headers_json(&request.headers),
            "timeout": request.timeout.as_secs_f64(),
            "spans": spans.iter().map(span_json).collect::<Vec<Value>>(),
        }));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default();
        ready(if reply.transport_error {
            Err(ScriptedError)
        } else {
            Ok(HttpResponse {
                status: reply.status,
            })
        })
    }
}

#[derive(Debug)]
pub(crate) struct RecordingSleep {
    pub(crate) journal: Arc<Journal>,
}

impl Sleep for RecordingSleep {
    fn sleep(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.journal.record_sleep(delay);
        Box::pin(ready(()))
    }
}

fn headers_json(headers: &HeaderMap) -> Value {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_lowercase(),
                value.to_str().map_or_else(|_| String::new(), str::to_owned),
            )
        })
        .collect();
    pairs.sort();
    Value::Array(
        pairs
            .into_iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}
