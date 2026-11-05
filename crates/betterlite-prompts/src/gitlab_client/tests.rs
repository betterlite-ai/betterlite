#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};

use secrecy::ExposeSecret as _;

use super::auth::RequestHeader;
use super::config::GitLabConfig;
use super::transport::{GitLabResponse, GitLabTransport};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedRequest {
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) max_body_bytes: usize,
}

#[derive(Clone, Default)]
pub(crate) struct FakeTransport {
    script: Arc<Mutex<VecDeque<Result<GitLabResponse, io::Error>>>>,
    log: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeTransport {
    pub(crate) fn scripted(responses: Vec<Result<GitLabResponse, io::Error>>) -> Self {
        Self {
            script: Arc::new(Mutex::new(VecDeque::from(responses))),
            log: Arc::default(),
        }
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.log.lock().unwrap().clone()
    }
}

impl GitLabTransport for FakeTransport {
    fn get(
        &self,
        url: &str,
        headers: &[RequestHeader],
        max_body_bytes: usize,
    ) -> impl Future<Output = Result<GitLabResponse, io::Error>> + Send {
        self.log.lock().unwrap().push(RecordedRequest {
            url: url.to_owned(),
            headers: headers
                .iter()
                .map(|header| {
                    (
                        header.name.to_owned(),
                        header.value.expose_secret().to_owned(),
                    )
                })
                .collect(),
            max_body_bytes,
        });
        ready(
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(io::Error::other("no scripted response left"))),
        )
    }
}

macro_rules! answer {
    ($status:expr, $body:expr $(,)?) => {
        Ok($crate::gitlab_client::transport::GitLabResponse {
            status: $status,
            body: String::from($body).into_bytes(),
        })
    };
}

pub(crate) use answer;

pub(crate) fn config_from(json_text: &str) -> GitLabConfig {
    serde_json::from_str(json_text).unwrap()
}
