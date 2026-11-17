use std::future::Future;
use std::pin::Pin;
use std::str;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::HeaderValue;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};

use super::records::{TinyfishRun, parse_run};

// LiteLLM passes the provider's pass-through credential as the Agent API's own header, sent lowercase on the wire.
pub const RUN_API_KEY_HEADER: &str = "x-api-key";
// LiteLLM builds the poll client with a 30-second timeout, so one dead poll cannot eat the whole budget.
pub const RUN_FETCH_TIMEOUT: Duration = Duration::from_secs(30);
// Screenshots are not needed to price a run and make the poll payload many times larger.
pub const RUN_SCREENSHOTS_QUERY: &str = "screenshots=none";
// A run id becomes one path segment, and these characters would leave it or end it early.
const REJECTED_RUN_ID_CHARACTERS: [char; 4] = ['/', '\\', '#', '?'];
const PARENT_SEGMENT: &str = "..";
// Python's quote(safe="") leaves the unreserved characters alone and percent-encodes every other byte.
const RUN_ID_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub struct RunRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub timeout: Duration,
}

pub struct RunReply {
    pub status: u16,
    pub body: Vec<u8>,
}

pub type RunFetchFuture<'call> =
    Pin<Box<dyn Future<Output = Result<RunReply, TransportError>> + Send + 'call>>;

// LiteLLM reads the key from passthrough_endpoint_router.get_credentials for every poll, since a rotated key must be picked up.
pub trait RunCredentials: Send + Sync {
    fn api_key(&self) -> Option<SecretString>;
}

pub trait RunTransport: Send + Sync {
    fn get(&self, request: RunRequest) -> RunFetchFuture<'_>;
}

// None is one failed poll: LiteLLM warns and lets the poller count it against its failure budget.
pub async fn fetch_run(
    credentials: &dyn RunCredentials,
    transport: &dyn RunTransport,
    api_base: &str,
    run_id: &str,
) -> Option<TinyfishRun> {
    let Some(api_key) = credentials.api_key() else {
        tracing::warn!("TinyFish passthrough: no API key available to poll run {run_id}");
        return None;
    };
    let Some(safe_run_id) = safe_run_id(run_id) else {
        tracing::warn!("TinyFish passthrough: invalid run_id {run_id:?}");
        return None;
    };
    let request = run_request(api_base, &safe_run_id, &api_key)?;
    match transport.get(request).await {
        Ok(reply) => reply_run(&safe_run_id, &reply),
        Err(error) => {
            tracing::warn!(%error, "[Non blocking logging error] TinyFish run fetch failed");
            None
        }
    }
}

// Python's quote(safe="") leaves the unreserved characters alone and percent-encodes the rest.
pub(super) fn safe_run_id(run_id: &str) -> Option<String> {
    let leaves_the_segment = REJECTED_RUN_ID_CHARACTERS
        .iter()
        .any(|character| run_id.contains(*character))
        || run_id.contains(PARENT_SEGMENT);
    if leaves_the_segment {
        return None;
    }
    Some(utf8_percent_encode(run_id, RUN_ID_ENCODE_SET).to_string())
}

// A key that cannot be a header value fails the poll inside LiteLLM's blanket handler, so it warns here too.
fn run_request(api_base: &str, safe_run_id: &str, api_key: &SecretString) -> Option<RunRequest> {
    let value = match HeaderValue::from_str(api_key.expose_secret()) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "[Non blocking logging error] TinyFish run fetch failed");
            return None;
        }
    };
    let mut headers = HeaderMap::new();
    headers.insert(RUN_API_KEY_HEADER, value);
    Some(RunRequest {
        url: format!("{api_base}/v1/runs/{safe_run_id}?{RUN_SCREENSHOTS_QUERY}"),
        headers,
        timeout: RUN_FETCH_TIMEOUT,
    })
}

// LiteLLM checks the status before reading the body, and a body that is not JSON fails the poll the same way.
fn reply_run(safe_run_id: &str, reply: &RunReply) -> Option<TinyfishRun> {
    if !(200..300).contains(&reply.status) {
        tracing::warn!(
            "TinyFish passthrough: GET /v1/runs/{safe_run_id} returned {}",
            reply.status
        );
        return None;
    }
    let Ok(text) = str::from_utf8(&reply.body) else {
        tracing::warn!(
            "[Non blocking logging error] TinyFish run fetch failed: the answer is not UTF-8"
        );
        return None;
    };
    let Ok(payload) = serde_json::from_str::<Json>(text) else {
        tracing::warn!(
            "[Non blocking logging error] TinyFish run fetch failed: the answer is not JSON"
        );
        return None;
    };
    parse_run(&payload)
}
