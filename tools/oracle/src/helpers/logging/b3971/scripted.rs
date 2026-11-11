use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_http::user_url::error::UserUrlError;
use betterlite_logging::finops::pointfive::upload::url_check::{UploadUrlCheck, ValidatedUpload};
use betterlite_logging::posthog_logging::delivery::{PostFailure, PostRequest, Transport};
use serde_json::{Map, Value, json};

use super::ReplyInput;

pub(super) struct AllowAllUrlCheck;

#[async_trait]
impl UploadUrlCheck for AllowAllUrlCheck {
    async fn check(&self, url: &str) -> Result<ValidatedUpload, UserUrlError> {
        Ok(ValidatedUpload {
            url: url.to_owned(),
            host_header: None,
        })
    }
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<u16>>,
    requests: Mutex<Vec<Value>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[ReplyInput]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().map(|reply| reply.status).collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn header_map(request: &PostRequest) -> Map<String, Value> {
    request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                Value::String(value.to_str().unwrap_or_default().to_owned()),
            )
        })
        .collect()
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<u16, PostFailure> {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let headers = header_map(&request);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"url": request.url, "headers": headers, "body": body}));
        let status = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(200);
        Ok(status)
    }
}
