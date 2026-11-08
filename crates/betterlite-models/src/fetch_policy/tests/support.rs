use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::str::FromStr as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::decisions::Json;
use crate::fetch_policy::client::{CatalogClient, CatalogRequest, FetchClientError};
use crate::fetch_policy::integrity::IntegrityPolicy;
use crate::fetch_policy::outcome::{
    AttemptOutcome, FetchResult, FetchedCatalog, Retryable, Unavailable,
};
use crate::fetch_policy::response::FetchResponse;
use crate::fetch_policy::retry::{CatalogSleeper, JitterSource};

pub(super) const URL: &str = "https://catalog.test/model.json";

pub(super) fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).unwrap()
}

pub(super) fn request() -> CatalogRequest {
    CatalogRequest {
        url: URL.to_owned(),
        timeout: Duration::from_secs(5),
    }
}

pub(super) fn response(status: u16, body: &str) -> FetchResponse {
    FetchResponse::new(status, BTreeMap::new(), body.as_bytes().to_vec())
}

pub(super) fn response_with_headers(
    status: u16,
    headers: &[(&str, &str)],
    body: &str,
) -> FetchResponse {
    let header_map = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    FetchResponse::new(status, header_map, body.as_bytes().to_vec())
}

pub(super) fn models(names: &[&str]) -> IndexMap<String, Json> {
    names
        .iter()
        .map(|name| ((*name).to_owned(), Json::Object(IndexMap::new())))
        .collect()
}

pub(super) fn policy(min_model_count: usize, max_shrink_ratio: &str) -> IntegrityPolicy {
    IntegrityPolicy {
        min_model_count,
        max_shrink_ratio: decimal(max_shrink_ratio),
    }
}

pub(super) fn retryable(retry_after: Option<&str>) -> Retryable {
    Retryable {
        reason: format!("HTTP 503 from {URL}"),
        retry_after_seconds: retry_after.map(decimal),
    }
}

pub(super) fn unavailable(outcome: AttemptOutcome) -> Unavailable {
    match outcome {
        AttemptOutcome::Unavailable(unavailable) => unavailable,
        AttemptOutcome::Reloaded(_) | AttemptOutcome::Retryable(_) => {
            panic!("expected unavailable")
        }
    }
}

pub(super) fn reloaded(result: FetchResult) -> FetchedCatalog {
    match result {
        FetchResult::Reloaded(fetched) => fetched,
        FetchResult::Unavailable(_) => panic!("expected reloaded"),
    }
}

pub(super) fn failed(result: FetchResult) -> Unavailable {
    match result {
        FetchResult::Unavailable(unavailable) => unavailable,
        FetchResult::Reloaded(_) => panic!("expected unavailable"),
    }
}

pub(super) struct ScriptedClient {
    replies: Vec<Result<FetchResponse, FetchClientError>>,
    calls: AtomicUsize,
}

impl ScriptedClient {
    pub(super) fn new(replies: Vec<Result<FetchResponse, FetchClientError>>) -> Self {
        Self {
            replies,
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn next_reply(&self) -> Result<FetchResponse, FetchClientError> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        self.replies.get(index).cloned().unwrap_or_else(|| {
            Err(FetchClientError::Transport {
                detail: "unscripted request".to_owned(),
            })
        })
    }
}

impl CatalogClient for ScriptedClient {
    fn get(&self, _request: &CatalogRequest) -> Result<FetchResponse, FetchClientError> {
        self.next_reply()
    }

    fn get_async(
        &self,
        _request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, FetchClientError>> + Send {
        ready(self.next_reply())
    }
}

pub(super) struct RecordingSleeper {
    seconds: Mutex<Vec<Decimal>>,
}

impl RecordingSleeper {
    pub(super) fn new() -> Self {
        Self {
            seconds: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seconds(&self) -> Vec<Decimal> {
        self.seconds.lock().unwrap().clone()
    }
}

impl CatalogSleeper for RecordingSleeper {
    async fn sleep(&self, seconds: Decimal) {
        self.seconds.lock().unwrap().push(seconds);
    }
}

pub(super) struct ScriptedJitter {
    values: VecDeque<Decimal>,
}

impl ScriptedJitter {
    pub(super) fn new(values: &[&str]) -> Self {
        Self {
            values: values.iter().map(|value| decimal(value)).collect(),
        }
    }
}

impl JitterSource for ScriptedJitter {
    fn unit_interval(&mut self) -> Decimal {
        self.values.pop_front().expect("scripted jitter exhausted")
    }
}
