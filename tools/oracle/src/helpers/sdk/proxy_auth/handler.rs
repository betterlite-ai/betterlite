use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, PoisonError};

use betterlite_sdk::proxy_auth::{AccessToken, ProxyAuthHandler, TokenCredential};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use super::Moment;
use crate::helpers::HelperResult;

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum Reply {
    Grant { token: String, expires_on: i64 },
    Refusal { refuse: String },
}

#[derive(Debug, Deserialize)]
struct At {
    at: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Handler {
    scope: String,
    replies: Vec<Reply>,
    calls: Vec<At>,
}

#[derive(Debug, Error)]
#[error("{0}")]
struct Refused(String);

struct Queue {
    replies: Mutex<VecDeque<Reply>>,
    scopes: Mutex<Vec<String>>,
}

impl TokenCredential for Queue {
    type Error = Refused;

    fn get_token(&self, scope: &str) -> impl Future<Output = Result<AccessToken, Refused>> + Send {
        self.scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(scope.to_owned());
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(match next {
            Some(Reply::Grant { token, expires_on }) => Ok(AccessToken {
                token: SecretString::from(token),
                expires_on,
            }),
            Some(Reply::Refusal { refuse }) => Err(Refused(refuse)),
            None => Err(Refused("no reply left".to_owned())),
        })
    }
}

pub(super) fn run(input: &Handler) -> HelperResult {
    let clock = Moment::default();
    let handler = ProxyAuthHandler {
        credential: Queue {
            replies: Mutex::new(input.replies.iter().cloned().collect()),
            scopes: Mutex::default(),
        },
        scope: input.scope.clone(),
        clock: clock.clone(),
        cached_token: Mutex::new(None),
    };
    let results: Vec<Value> = super::runtime()?.block_on(async {
        let mut results = Vec::new();
        for call in &input.calls {
            clock.0.store(call.at, Ordering::SeqCst);
            let outcome = handler.get_auth_headers().await;
            results.push(outcome.map_or_else(
                |_| json!({"error": true}),
                |headers| {
                    let bearer = headers.get("authorization").map(|value| value.to_str());
                    json!({"headers": {"Authorization": bearer.and_then(Result::ok)}})
                },
            ));
        }
        results
    });
    let scopes = handler
        .credential
        .scopes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({"results": results, "credential_scopes": scopes}))
}
