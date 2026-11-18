use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_openai::output_limits::raw_send::{
    AsyncRawChatTransport, BadRequestReply, RawChatError, RawChatResponse, StatusReply,
    SyncRawChatTransport,
};
use serde_json::{Map, Value};

use super::cases::AzureOutcome;
use super::project::header_map;

pub(crate) struct Scripted {
    pub seen: Mutex<Vec<Value>>,
    pub outcome: Box<AzureOutcome>,
}

impl Scripted {
    fn reply(&self, data: &Map<String, Value>) -> Result<RawChatResponse, RawChatError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Value::Object(data.clone()));
        match self.outcome.as_ref() {
            AzureOutcome::Reply { headers, body } => Ok(RawChatResponse {
                headers: header_map(headers),
                body: body.clone(),
            }),
            AzureOutcome::BadRequest { headers, body } => {
                Err(RawChatError::BadRequest(Box::new(BadRequestReply {
                    message: error_message(body),
                    headers: header_map(headers),
                    body: body.clone(),
                })))
            }
            AzureOutcome::Status {
                status,
                headers,
                body,
            } => Err(RawChatError::Status(Box::new(StatusReply {
                status: *status,
                message: error_message(body),
                headers: header_map(headers),
                // the SDK's exception body is the response's error member, or the whole body when it has none
                body: body.get("error").cloned().unwrap_or_else(|| body.clone()),
            }))),
            AzureOutcome::Timeout => Err(RawChatError::Timeout {
                message: String::from("Request timed out."),
            }),
        }
    }
}

impl SyncRawChatTransport for Scripted {
    fn create(
        &self,
        data: &Map<String, Value>,
        _timeout: f64,
    ) -> Result<RawChatResponse, RawChatError> {
        self.reply(data)
    }
}

#[async_trait]
impl AsyncRawChatTransport for Scripted {
    async fn create(
        &self,
        data: &Map<String, Value>,
        _timeout: f64,
    ) -> Result<RawChatResponse, RawChatError> {
        self.reply(data)
    }
}

// the sdk surfaces the provider's own sentence, which is what the marker is matched against
fn error_message(body: &Value) -> String {
    body.get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map_or_else(|| body.to_string(), str::to_owned)
}

pub(crate) fn recorded(transport: &Scripted) -> Value {
    let seen = transport
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Value::Array(seen.clone())
}
