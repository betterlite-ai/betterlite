use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::runway::tasks::poll::TaskTransport;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde_json::{Value, json};
use url::Url;

use super::Reply;

pub(super) struct Script {
    replies: Mutex<VecDeque<(u16, String)>>,
    requests: Mutex<Vec<(String, HeaderMap)>>,
}

impl Script {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .iter()
                    .map(|reply| (reply.status, reply.body.clone()))
                    .collect(),
            ),
            requests: Mutex::default(),
        }
    }

    pub(super) fn describe(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(url, headers)| {
                json!({
                    "url": url,
                    "headers": {
                        "authorization": header_text(headers, AUTHORIZATION.as_str()),
                        "x-runway-version": header_text(headers, "x-runway-version"),
                    },
                })
            })
            .collect()
    }
}

impl TaskTransport for Script {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_string(), headers));
        let (status, body) = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or((500, String::new()));
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}
