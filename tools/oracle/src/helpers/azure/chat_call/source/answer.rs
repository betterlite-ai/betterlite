use std::collections::BTreeMap;

use betterlite_openai::output_limits::raw_send::{
    BadRequestReply, RawChatError, RawChatResponse, StatusReply,
};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::Value;

use super::CHAT_BODY;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    Status,
    BadRequest,
    Timeout,
    Other,
    Pending,
}

// The answer the fixture transport hands the raw-send wrappers
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Answer {
    #[serde(default)]
    pub(super) error: Option<ErrorKind>,
    #[serde(default)]
    pub(super) status: Option<u16>,
    #[serde(default)]
    pub(super) message: Option<String>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) body: Option<String>,
}

impl Answer {
    fn headers(&self) -> HeaderMap {
        let mut pairs = if self.headers.is_empty() {
            BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())])
        } else {
            self.headers.clone()
        };
        // httpx stamps the body length on every response the fixture transport builds
        let length = self
            .body
            .clone()
            .unwrap_or_else(|| CHAT_BODY.to_owned())
            .len();
        pairs
            .entry("content-length".to_owned())
            .or_insert_with(|| length.to_string());
        let mut map = HeaderMap::new();
        for (raw_name, raw_value) in &pairs {
            if let (Ok(name), Ok(value)) = (
                raw_name.parse::<HeaderName>(),
                raw_value.parse::<HeaderValue>(),
            ) {
                map.insert(name, value);
            }
        }
        map
    }

    pub(super) fn make(&self) -> Result<RawChatResponse, RawChatError> {
        let text = self.body.clone().unwrap_or_else(|| CHAT_BODY.to_owned());
        let body =
            serde_json::from_str(&text).unwrap_or_else(|_unparsed| Value::String(text.clone()));
        let message = self.message.clone().unwrap_or_default();
        match self.error {
            None => Ok(RawChatResponse {
                headers: self.headers(),
                body,
            }),
            Some(ErrorKind::Status) => Err(RawChatError::Status(Box::new(StatusReply {
                status: self.status.unwrap_or(400),
                message,
                headers: self.headers(),
                body,
            }))),
            Some(ErrorKind::BadRequest) => {
                Err(RawChatError::BadRequest(Box::new(BadRequestReply {
                    message,
                    headers: self.headers(),
                    body,
                })))
            }
            Some(ErrorKind::Timeout) => Err(RawChatError::Timeout { message }),
            Some(ErrorKind::Other) => Err(RawChatError::Other { message }),
            Some(ErrorKind::Pending) => Err(RawChatError::Other {
                message: "the scripted answer never arrives on the sync transport".to_owned(),
            }),
        }
    }
}
