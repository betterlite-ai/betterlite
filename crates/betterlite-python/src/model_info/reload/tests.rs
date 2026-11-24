#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::fetch_policy::client::{CatalogClient, CatalogRequest};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fetch_policy::retry::CatalogSleeper;
use betterlite_models::fetch_policy::revision::git_blob_id;
use indexmap::IndexMap;
use pyo3_async_runtimes::tokio::get_runtime;
use rust_decimal::Decimal;
use serde_json::value::RawValue;

use super::{reload_async, reload_async_with, reload_blocking, reload_blocking_with};
use crate::model_info::convert::ReloadQuery;
use crate::model_info::fetch::FetchError;
use crate::model_info::state::Runtime;
use crate::native_api::bundled_model_cost_json;

const URL: &str = "https://example.test/map.json";

struct ScriptedClient {
    responses: Mutex<VecDeque<Result<FetchResponse, FetchError>>>,
    calls: AtomicU32,
}

impl ScriptedClient {
    fn new(responses: Vec<Result<FetchResponse, FetchError>>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            calls: AtomicU32::new(0),
        }
    }

    fn next(&self) -> Result<FetchResponse, FetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut responses = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        responses
            .pop_front()
            .unwrap_or_else(|| Err(FetchError::Transport(TransportError::Closed)))
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl CatalogClient for ScriptedClient {
    type Error = FetchError;

    fn get(&self, _request: &CatalogRequest) -> Result<FetchResponse, FetchError> {
        self.next()
    }

    fn get_async(
        &self,
        _request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, FetchError>> + Send {
        let outcome = self.next();
        async move { outcome }
    }
}

struct RecordingSleeper {
    waits: Mutex<Vec<Decimal>>,
}

impl RecordingSleeper {
    fn new() -> Self {
        Self {
            waits: Mutex::new(Vec::new()),
        }
    }

    fn waits(&self) -> Vec<Decimal> {
        self.waits.lock().unwrap().clone()
    }
}

impl CatalogSleeper for RecordingSleeper {
    fn sleep(&self, seconds: Decimal) -> impl Future<Output = ()> + Send {
        self.waits.lock().unwrap().push(seconds);
        async {}
    }
}

fn runtime() -> Runtime {
    let text = bundled_model_cost_json().unwrap();
    Runtime::new(&text).unwrap()
}

fn query() -> ReloadQuery {
    ReloadQuery {
        url: URL.to_owned(),
        timeout: 5.0,
        max_attempts: 3,
    }
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn local_environment() -> BTreeMap<String, String> {
    BTreeMap::from([("LITELLM_LOCAL_MODEL_COST_MAP".to_owned(), "true".to_owned())])
}

fn body_with_aliases() -> Vec<u8> {
    let text = bundled_model_cost_json().unwrap();
    let mut map: IndexMap<String, Box<RawValue>> = serde_json::from_str(&text).unwrap();
    let entry = RawValue::from_string(
        r#"{"litellm_provider":"openai","aliases":["acme-alias"]}"#.to_owned(),
    )
    .unwrap();
    map.insert("acme-canonical".to_owned(), entry);
    serde_json::to_vec(&map).unwrap()
}

fn response(status: u16, headers: &[(&str, &str)], body: &[u8]) -> FetchResponse {
    FetchResponse::new(
        status,
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body.to_vec(),
    )
}

fn envelope(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn source(runtime: &Runtime) -> IndexMap<String, Json> {
    serde_json::from_str(&runtime.source_json().unwrap()).unwrap()
}

#[test]
fn model_info_reload_blocking_returns_the_fetched_map() {
    let runtime = runtime();
    let body = body_with_aliases();
    let client = ScriptedClient::new(vec![Ok(response(200, &[("etag", "etag-1")], &body))]);
    let answer = get_runtime()
        .block_on(reload_blocking_with(
            &runtime,
            &query(),
            &client,
            &environment(),
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("reloaded".to_owned()))
    );
    assert_eq!(
        parsed.get("revision"),
        Some(&Json::Text(git_blob_id(&body)))
    );
    assert_eq!(parsed.get("etag"), Some(&Json::Text("etag-1".to_owned())));
    let Some(Json::Object(models)) = parsed.get("model_cost_map") else {
        panic!("model_cost_map is not an object");
    };
    assert!(models.contains_key("acme-alias"));
    assert!(models.contains_key("acme-canonical"));
    assert!(!models.contains_key("fallback_generalizations"));
    assert_eq!(client.calls(), 1);
    let info = source(&runtime);
    assert_eq!(info.get("source"), Some(&Json::Text("remote".to_owned())));
    assert_eq!(info.get("url"), Some(&Json::Text(URL.to_owned())));
    assert_eq!(info.get("is_env_forced"), Some(&Json::Bool(false)));
    assert_eq!(info.get("fallback_reason"), Some(&Json::Null));
    assert_eq!(
        info.get("source_revision"),
        Some(&Json::Text(git_blob_id(&body)))
    );
    assert_eq!(info.get("etag"), Some(&Json::Text("etag-1".to_owned())));
}

#[test]
fn model_info_reload_blocking_falls_back_on_a_retryable_status() {
    let runtime = runtime();
    let client = ScriptedClient::new(vec![Ok(response(429, &[("Retry-After", "0")], b"{}"))]);
    let answer = get_runtime()
        .block_on(reload_blocking_with(
            &runtime,
            &query(),
            &client,
            &environment(),
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("reloaded".to_owned()))
    );
    let bundled_revision = runtime.bundled.revision.clone();
    assert_eq!(
        parsed.get("revision"),
        Some(&Json::Text(bundled_revision.clone().unwrap()))
    );
    assert_eq!(parsed.get("etag"), Some(&Json::Null));
    assert_eq!(client.calls(), 1);
    let info = source(&runtime);
    assert_eq!(info.get("source"), Some(&Json::Text("local".to_owned())));
    assert_eq!(info.get("url"), Some(&Json::Text(URL.to_owned())));
    assert_eq!(info.get("is_env_forced"), Some(&Json::Bool(false)));
    assert_eq!(
        info.get("fallback_reason"),
        Some(&Json::Text(format!(
            "Remote fetch failed: HTTP 429 from {URL}"
        )))
    );
    assert_eq!(
        info.get("source_revision"),
        Some(&Json::Text(bundled_revision.unwrap()))
    );
    assert_eq!(info.get("etag"), Some(&Json::Null));
}

#[test]
fn model_info_reload_blocking_falls_back_on_a_corrupt_body() {
    let runtime = runtime();
    let client = ScriptedClient::new(vec![Ok(response(200, &[], b"{not json"))]);
    let answer = get_runtime()
        .block_on(reload_blocking_with(
            &runtime,
            &query(),
            &client,
            &environment(),
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("reloaded".to_owned()))
    );
    let info = source(&runtime);
    assert_eq!(info.get("source"), Some(&Json::Text("local".to_owned())));
    let Some(Json::Text(reason)) = info.get("fallback_reason") else {
        panic!("fallback_reason is not text");
    };
    assert!(reason.starts_with(&format!("Remote fetch failed: invalid JSON from {URL}")));
}

#[test]
fn model_info_reload_blocking_falls_back_on_an_integrity_failure() {
    let runtime = runtime();
    let client = ScriptedClient::new(vec![Ok(response(
        200,
        &[],
        br#"{"acme-one":{"litellm_provider":"openai"}}"#,
    ))]);
    let answer = get_runtime()
        .block_on(reload_blocking_with(
            &runtime,
            &query(),
            &client,
            &environment(),
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("reloaded".to_owned()))
    );
    let info = source(&runtime);
    assert_eq!(
        info.get("fallback_reason"),
        Some(&Json::Text(
            "Remote data failed integrity validation".to_owned()
        ))
    );
}

#[test]
fn model_info_reload_blocking_uses_the_local_env_flag() {
    let runtime = runtime();
    let client = ScriptedClient::new(Vec::new());
    let answer = get_runtime()
        .block_on(reload_blocking_with(
            &runtime,
            &query(),
            &client,
            &local_environment(),
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("reloaded".to_owned()))
    );
    assert_eq!(client.calls(), 0);
    let info = source(&runtime);
    assert_eq!(info.get("source"), Some(&Json::Text("local".to_owned())));
    assert_eq!(info.get("url"), Some(&Json::Null));
    assert_eq!(info.get("is_env_forced"), Some(&Json::Bool(true)));
    assert_eq!(info.get("fallback_reason"), Some(&Json::Null));
    assert_eq!(
        info.get("source_revision"),
        Some(&Json::Text(runtime.bundled.revision.unwrap()))
    );
}

#[test]
fn model_info_reload_async_retries_a_retryable_status() {
    let runtime = runtime();
    let body = body_with_aliases();
    let client = ScriptedClient::new(vec![
        Ok(response(429, &[("Retry-After", "0")], b"{}")),
        Ok(response(200, &[("etag", "etag-2")], &body)),
    ]);
    let sleeper = RecordingSleeper::new();
    let answer = get_runtime()
        .block_on(reload_async_with(
            &runtime,
            &query(),
            &client,
            &environment(),
            &sleeper,
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("reloaded".to_owned()))
    );
    assert_eq!(
        parsed.get("revision"),
        Some(&Json::Text(git_blob_id(&body)))
    );
    assert_eq!(parsed.get("etag"), Some(&Json::Text("etag-2".to_owned())));
    assert_eq!(client.calls(), 2);
    assert_eq!(sleeper.waits(), vec![Decimal::ZERO]);
    let info = source(&runtime);
    assert_eq!(info.get("source"), Some(&Json::Text("remote".to_owned())));
    assert_eq!(info.get("url"), Some(&Json::Text(URL.to_owned())));
    assert_eq!(info.get("is_env_forced"), Some(&Json::Bool(false)));
    assert_eq!(info.get("fallback_reason"), Some(&Json::Null));
    assert_eq!(
        info.get("source_revision"),
        Some(&Json::Text(git_blob_id(&body)))
    );
    assert_eq!(info.get("etag"), Some(&Json::Text("etag-2".to_owned())));
}

#[test]
fn model_info_reload_async_returns_unavailable_after_the_attempts() {
    let runtime = runtime();
    let before = runtime.source_json().unwrap();
    let client = ScriptedClient::new(vec![
        Ok(response(500, &[], b"")),
        Ok(response(500, &[], b"")),
    ]);
    let sleeper = RecordingSleeper::new();
    let mut short = query();
    short.max_attempts = 2;
    let answer = get_runtime()
        .block_on(reload_async_with(
            &runtime,
            &short,
            &client,
            &environment(),
            &sleeper,
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("unavailable".to_owned()))
    );
    assert_eq!(
        parsed.get("reason"),
        Some(&Json::Text(format!(
            "HTTP 500 from {URL} (after 2 attempts)"
        )))
    );
    assert_eq!(client.calls(), 2);
    assert_eq!(sleeper.waits().len(), 1);
    assert_eq!(runtime.source_json().unwrap(), before);
}

#[test]
fn model_info_reload_async_rejects_an_integrity_failure() {
    let runtime = runtime();
    let before = runtime.source_json().unwrap();
    let client = ScriptedClient::new(vec![Ok(response(
        200,
        &[],
        br#"{"acme-one":{"litellm_provider":"openai"}}"#,
    ))]);
    let sleeper = RecordingSleeper::new();
    let answer = get_runtime()
        .block_on(reload_async_with(
            &runtime,
            &query(),
            &client,
            &environment(),
            &sleeper,
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("unavailable".to_owned()))
    );
    assert_eq!(
        parsed.get("reason"),
        Some(&Json::Text(format!(
            "model cost map from {URL} failed integrity validation"
        )))
    );
    assert_eq!(runtime.source_json().unwrap(), before);
}

#[test]
fn model_info_reload_blocking_answers_through_the_process_environment() {
    let runtime = runtime();
    let mut broken = query();
    broken.url = "not a url".to_owned();
    let answer = get_runtime()
        .block_on(reload_blocking(&runtime, &broken))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("reloaded".to_owned()))
    );
    let info = source(&runtime);
    assert_eq!(info.get("source"), Some(&Json::Text("local".to_owned())));
}

#[test]
fn model_info_reload_async_answers_through_the_process_environment() {
    let runtime = runtime();
    let before = runtime.source_json().unwrap();
    let mut broken = query();
    broken.url = "not a url".to_owned();
    let answer = get_runtime()
        .block_on(reload_async(&runtime, &broken))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("unavailable".to_owned()))
    );
    let Some(Json::Text(reason)) = parsed.get("reason") else {
        panic!("reason is not text");
    };
    assert!(reason.contains("fetching not a url"));
    assert_eq!(runtime.source_json().unwrap(), before);
}

#[test]
fn model_info_reload_async_uses_the_local_env_flag() {
    let runtime = runtime();
    let client = ScriptedClient::new(Vec::new());
    let sleeper = RecordingSleeper::new();
    let answer = get_runtime()
        .block_on(reload_async_with(
            &runtime,
            &query(),
            &client,
            &local_environment(),
            &sleeper,
        ))
        .unwrap();
    let parsed = envelope(&answer);
    assert_eq!(
        parsed.get("status"),
        Some(&Json::Text("reloaded".to_owned()))
    );
    assert_eq!(client.calls(), 0);
    assert_eq!(sleeper.waits(), Vec::new());
    let info = source(&runtime);
    assert_eq!(info.get("is_env_forced"), Some(&Json::Bool(true)));
    assert_eq!(info.get("url"), Some(&Json::Null));
}
