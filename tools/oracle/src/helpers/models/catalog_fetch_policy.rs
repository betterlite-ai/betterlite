use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::str::FromStr as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::fetch_policy::client::{CatalogClient, CatalogRequest, FetchClientError};
use betterlite_models::fetch_policy::integrity::{
    IntegrityPolicy, MAX_SHRINK_RATIO, MIN_MODEL_COUNT, check_model_count_not_reduced,
    count_model_entries, validate_model_cost_map,
};
use betterlite_models::fetch_policy::outcome::{
    AttemptOutcome, FetchResult, Retryable, classify_fetch_response,
};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fetch_policy::retry::{
    CatalogSleeper, JitterSource, NextStep, fetch_with_retry, next_retry_step,
    parse_retry_after_seconds, retry_wait_seconds,
};
use betterlite_models::fetch_policy::revision::git_blob_id;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    body: String,
    #[serde(default)]
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    model_keys: Vec<String>,
    #[serde(default)]
    backup_model_count: usize,
    #[serde(default)]
    min_model_count: Option<usize>,
    #[serde(default)]
    max_shrink_ratio: Option<String>,
    #[serde(default)]
    retry_after: Option<String>,
    #[serde(default)]
    attempt: u32,
    #[serde(default)]
    max_attempts: u32,
    #[serde(default)]
    jitter: Vec<String>,
    #[serde(default)]
    replies: Vec<ReplyInput>,
    #[serde(default)]
    timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyInput {
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    RawBlobRevision,
    ValidateMap,
    ClassifyResponse,
    ParseRetryAfter,
    RetryWait,
    NextRetryWait,
    FetchWithRetry,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input.operation {
            Operation::RawBlobRevision => Ok(json!({
                "revision": git_blob_id(input.body.as_bytes()),
            })),
            Operation::ValidateMap => run_validate(input),
            Operation::ClassifyResponse => Ok(run_classify(input)),
            Operation::ParseRetryAfter => Ok(run_parse_retry_after(input)),
            Operation::RetryWait => run_retry_wait(input),
            Operation::NextRetryWait => run_next_retry_wait(input),
            Operation::FetchWithRetry => run_fetch(input),
        }
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn optional_decimal(text: Option<&str>) -> Result<Option<Decimal>, Box<HelperError>> {
    text.map_or(Ok(None), |value| {
        Decimal::from_str(value)
            .map(Some)
            .map_err(|error| invalid(format!("{value}: {error}")))
    })
}

struct ScriptedJitter {
    values: VecDeque<Decimal>,
}

impl ScriptedJitter {
    fn new(values: &[String]) -> Result<Self, Box<HelperError>> {
        let parsed = values
            .iter()
            .map(|text| {
                Decimal::from_str(text).map_err(|error| invalid(format!("{text}: {error}")))
            })
            .collect::<Result<VecDeque<_>, _>>()?;
        Ok(Self { values: parsed })
    }
}

impl JitterSource for ScriptedJitter {
    fn unit_interval(&mut self) -> Decimal {
        self.values.pop_front().unwrap_or(Decimal::ZERO)
    }
}

fn integrity_policy(input: &Input) -> Result<IntegrityPolicy, Box<HelperError>> {
    let min_model_count = input.min_model_count.unwrap_or(MIN_MODEL_COUNT);
    let max_shrink_ratio = match input.max_shrink_ratio.as_deref() {
        Some(text) => {
            Decimal::from_str(text).map_err(|error| invalid(format!("{text}: {error}")))?
        }
        None => MAX_SHRINK_RATIO,
    };
    Ok(IntegrityPolicy {
        min_model_count,
        max_shrink_ratio,
    })
}

fn model_keys(input: &Input) -> IndexMap<String, Json> {
    input
        .model_keys
        .iter()
        .map(|key| (key.clone(), Json::Object(IndexMap::new())))
        .collect()
}

fn run_validate(input: &Input) -> HelperResult {
    let models = model_keys(input);
    let policy = integrity_policy(input)?;
    let valid = validate_model_cost_map(&models, input.backup_model_count, policy).is_ok();
    let count_ok = check_model_count_not_reduced(&models, input.backup_model_count, policy).is_ok();
    Ok(json!({
        "valid": valid,
        "is_valid_dict": !models.is_empty(),
        "count_not_reduced": count_ok,
        "count": count_model_entries(&models),
    }))
}

fn sorted_models(models: &IndexMap<String, Json>) -> Vec<String> {
    let mut names = models.keys().cloned().collect::<Vec<_>>();
    names.sort_unstable();
    names
}

fn run_classify(input: &Input) -> Value {
    let response = FetchResponse::new(
        input.status,
        input.headers.clone(),
        input.body.as_bytes().to_vec(),
    );
    match classify_fetch_response(response, &input.url) {
        AttemptOutcome::Reloaded(fetched) => json!({
            "kind": "reloaded",
            "revision": fetched.revision,
            "etag": fetched.etag,
            "models": sorted_models(&fetched.models),
        }),
        AttemptOutcome::Unavailable(_) => json!({"kind": "unavailable"}),
        AttemptOutcome::Retryable(retryable) => json!({
            "kind": "retryable",
            "retry_after_seconds": retryable
                .retry_after_seconds
                .map(|seconds| seconds.normalize().to_string()),
        }),
    }
}

fn run_parse_retry_after(input: &Input) -> Value {
    let response = FetchResponse::new(200, input.headers.clone(), Vec::new());
    json!({
        "retry_after_seconds": parse_retry_after_seconds(&response)
            .map(|seconds| seconds.normalize().to_string()),
    })
}

fn run_retry_wait(input: &Input) -> HelperResult {
    let retryable = Retryable {
        reason: "synthetic".to_owned(),
        retry_after_seconds: optional_decimal(input.retry_after.as_deref())?,
    };
    let mut jitter = ScriptedJitter::new(&input.jitter)?;
    let wait = retry_wait_seconds(&retryable, input.attempt, &mut jitter);
    Ok(json!({"wait_seconds": wait.normalize().to_string()}))
}

fn run_next_retry_wait(input: &Input) -> HelperResult {
    let retryable = Retryable {
        reason: "synthetic".to_owned(),
        retry_after_seconds: optional_decimal(input.retry_after.as_deref())?,
    };
    let mut jitter = ScriptedJitter::new(&input.jitter)?;
    let step = next_retry_step(
        AttemptOutcome::Retryable(retryable),
        input.attempt,
        input.max_attempts,
        &mut jitter,
    );
    match step {
        NextStep::Retry(seconds) => Ok(json!({
            "kind": "retry",
            "wait_seconds": seconds.normalize().to_string(),
        })),
        NextStep::Done(FetchResult::Unavailable(_)) => Ok(json!({"kind": "unavailable"})),
        NextStep::Done(FetchResult::Reloaded(_)) => {
            Err(invalid("a retryable outcome never reloads".to_owned()))
        }
    }
}

struct ScriptedClient {
    replies: Vec<Result<FetchResponse, FetchClientError>>,
    calls: AtomicUsize,
}

impl ScriptedClient {
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

struct RecordingSleeper {
    seconds: Mutex<Vec<Decimal>>,
}

impl RecordingSleeper {
    fn recorded(&self) -> Vec<Decimal> {
        match self.seconds.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl CatalogSleeper for RecordingSleeper {
    fn sleep(&self, seconds: Decimal) -> impl Future<Output = ()> + Send {
        match self.seconds.lock() {
            Ok(mut guard) => guard.push(seconds),
            Err(poisoned) => poisoned.into_inner().push(seconds),
        }
        ready(())
    }
}

fn scripted_replies(replies: &[ReplyInput]) -> Vec<Result<FetchResponse, FetchClientError>> {
    replies
        .iter()
        .map(|reply| {
            Ok(FetchResponse::new(
                reply.status,
                reply.headers.clone(),
                reply.body.as_bytes().to_vec(),
            ))
        })
        .collect()
}

fn fetch_output(result: FetchResult, requests: usize, sleeps: &[String]) -> Value {
    match result {
        FetchResult::Reloaded(fetched) => json!({
            "kind": "reloaded",
            "requests": requests,
            "sleeps": sleeps,
            "revision": fetched.revision,
            "etag": fetched.etag,
            "models": sorted_models(&fetched.models),
        }),
        FetchResult::Unavailable(_) => json!({
            "kind": "unavailable",
            "requests": requests,
            "sleeps": sleeps,
        }),
    }
}

fn run_fetch(input: &Input) -> HelperResult {
    let client = ScriptedClient {
        replies: scripted_replies(&input.replies),
        calls: AtomicUsize::new(0),
    };
    let sleeper = RecordingSleeper {
        seconds: Mutex::new(Vec::new()),
    };
    let mut jitter = ScriptedJitter::new(&input.jitter)?;
    let request = CatalogRequest {
        url: input.url.clone(),
        timeout: Duration::from_secs(input.timeout.unwrap_or(5)),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    let result = runtime.block_on(fetch_with_retry(
        &request,
        input.max_attempts,
        &client,
        &sleeper,
        &mut jitter,
    ));
    let sleeps = sleeper
        .recorded()
        .iter()
        .map(|seconds| seconds.normalize().to_string())
        .collect::<Vec<_>>();
    Ok(fetch_output(
        result,
        client.calls.load(Ordering::SeqCst),
        &sleeps,
    ))
}
