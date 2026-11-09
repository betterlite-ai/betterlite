use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::agents::json::AgentJson;
use betterlite_http::user_url::error::{SsrfError, UserUrlError};
use betterlite_server::api::a2a_discover::discovery::{
    CardResponse, CardSource, CardTransportError, DiscoveryMode, fetch_well_known_card,
};
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Urls,
    Card,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    mode: Option<DiscoveryMode>,
    #[serde(default)]
    params: Option<BTreeMap<String, AgentJson>>,
    #[serde(default)]
    replies: Vec<Reply>,
}

#[derive(Clone, Debug, Deserialize)]
struct Reply {
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    error: Option<ReplyFailure>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplyFailure {
    Ssrf,
    Transport,
}

struct Script {
    replies: Vec<Reply>,
    urls: Mutex<Vec<String>>,
}

impl Script {
    fn recorded_urls(&self) -> Vec<String> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CardSource for Script {
    async fn get(&self, url: &str) -> Result<CardResponse, CardTransportError> {
        let index = {
            let mut urls = self.urls.lock().unwrap_or_else(PoisonError::into_inner);
            let index = urls.len();
            urls.push(url.to_owned());
            index
        };
        match self.replies.get(index) {
            Some(Reply {
                error: Some(ReplyFailure::Ssrf),
                ..
            }) => Err(CardTransportError::Url(UserUrlError::Ssrf(
                SsrfError::BlockedAddress {
                    address: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
                },
            ))),
            Some(Reply {
                error: Some(ReplyFailure::Transport),
                ..
            })
            | None => Err(CardTransportError::Deadline {
                timeout: Duration::from_secs(10),
            }),
            Some(Reply {
                status,
                body,
                error: None,
            }) => Ok(CardResponse {
                status: status.unwrap_or(200),
                body: body.clone().into_bytes(),
            }),
        }
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .map_err(|error| helper_error("RuntimeError", error.to_string()))?;
        let script = Script {
            replies: input.replies.clone(),
            urls: Mutex::new(Vec::new()),
        };
        let base_url = input.base_url.as_deref().unwrap_or_default();
        let params = input.params.clone().unwrap_or_default();
        let result = runtime.block_on(fetch_well_known_card(
            &script,
            base_url,
            input.mode.unwrap_or_default(),
            &params,
        ));
        let urls = script.recorded_urls();
        if matches!(input.operation, Operation::Urls) && !urls.is_empty() {
            return serde_json::to_value(urls)
                .map_err(|error| helper_error("SerializationError", error.to_string()));
        }
        match result {
            Ok(card) => Ok(card),
            Err(error) => Err(helper_error("AgentCardDiscoveryError", error.to_string())),
        }
    }
}
