#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use http::HeaderMap;
use rstest::rstest;

use super::completion::{Replicate, ReplicateCall};
use super::stream::poll_stream;
use super::transport::{ReplicateRequest, ReplicateResponse, ReplicateTransport};
use super::{
    OutputPolicy, ReplicateChunk, ReplicateCompletion, ReplicatePolling, ReplicateSleeper,
};
use crate::oci_common::Clock;
use crate::replicate::error::ReplicateFailure;
use crate::replicate::reply::{PostCall, PostCallLogger, ReplicateReply, TokenCounter};
use crate::replicate::request::ReplicateDefaults;

const CREATE: &str = r#"{"urls":{"get":"https://poll/1"}}"#;
// differs from now() so a reply stamped from the call time instead of the clock shows
const CLOCK_SECONDS: i64 = 1_700_000_500;
const MODEL: &str = "meta-llama/llama-2-70b-chat";

type Events = Arc<Mutex<Vec<&'static str>>>;

#[derive(Clone)]
struct ScriptedResponse {
    status: u16,
    body: String,
}

fn reply(status: u16, body: &str) -> ScriptedResponse {
    ScriptedResponse {
        status,
        body: String::from(body),
    }
}

fn reply_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-replicate", "1".parse().unwrap());
    headers
}

struct ScriptedTransport {
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Mutex<Vec<ReplicateRequest>>,
    events: Events,
}

impl ScriptedTransport {
    fn new(responses: Vec<ScriptedResponse>, events: Events) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            requests: Mutex::new(Vec::new()),
            events,
        }
    }

    fn requests(&self) -> Vec<ReplicateRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ReplicateTransport for ScriptedTransport {
    async fn send(&self, request: ReplicateRequest) -> Result<ReplicateResponse, TransportError> {
        self.events.lock().unwrap().push("send");
        self.requests.lock().unwrap().push(request);
        let next = self.responses.lock().unwrap().pop_front();
        match next {
            Some(response) => Ok(ReplicateResponse {
                status: response.status,
                headers: reply_headers(),
                body: response.body.into_bytes(),
            }),
            None => Err(TransportError::Closed),
        }
    }
}

struct RecordingSleeper {
    delays: Mutex<Vec<f64>>,
}

impl RecordingSleeper {
    fn new() -> Self {
        Self {
            delays: Mutex::new(Vec::new()),
        }
    }

    fn delays(&self) -> Vec<f64> {
        self.delays.lock().unwrap().clone()
    }
}

#[async_trait]
impl ReplicateSleeper for RecordingSleeper {
    async fn sleep(&self, seconds: f64) {
        self.delays.lock().unwrap().push(seconds);
    }
}

#[derive(Default)]
struct Counting {
    message_models: Mutex<Vec<String>>,
    responses: Mutex<Vec<(String, String)>>,
}

impl Counting {
    fn message_models(&self) -> Vec<String> {
        self.message_models.lock().unwrap().clone()
    }

    fn responses(&self) -> Vec<(String, String)> {
        self.responses.lock().unwrap().clone()
    }
}

#[async_trait]
impl TokenCounter for Counting {
    async fn count_messages(&self, model: &str, messages: &[Message]) -> Result<i64, CountError> {
        self.message_models
            .lock()
            .unwrap()
            .push(String::from(model));
        i64::try_from(messages.len()).map_err(|_overflow| CountError::Overflow)
    }

    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<i64, CountError> {
        self.responses
            .lock()
            .unwrap()
            .push((String::from(model), String::from(text)));
        i64::try_from(text.chars().count()).map_err(|_overflow| CountError::Overflow)
    }
}

struct ProbeClock {
    time: DateTime<Utc>,
    events: Events,
}

impl Clock for ProbeClock {
    fn now(&self) -> DateTime<Utc> {
        self.events.lock().unwrap().push("clock");
        self.time
    }
}

struct SilentLogger;

impl PostCallLogger for SilentLogger {
    fn post_call(&self, _call: PostCall<'_>) {}
}

struct Harness {
    prompts: PromptFactory,
    counter: Counting,
    logger: SilentLogger,
    clock: ProbeClock,
    events: Events,
}

impl Harness {
    fn new() -> Self {
        let events = Events::default();
        Self {
            prompts: PromptFactory {
                cache: Mutex::new(builtin_entries()),
                hub: Hub::Unreachable,
            },
            counter: Counting::default(),
            logger: SilentLogger,
            clock: ProbeClock {
                time: DateTime::from_timestamp(CLOCK_SECONDS, 0).unwrap(),
                events: Arc::clone(&events),
            },
            events,
        }
    }
}

fn settings(retries: i32, retry_delay: f64) -> ReplicatePolling {
    ReplicatePolling {
        retries,
        retry_delay_seconds: retry_delay,
        polling_delay_seconds: 0.5,
        model_name_with_id_length: 64,
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap()
}

fn bit_pattern(delays: &[f64]) -> Vec<u64> {
    delays.iter().map(|delay| delay.to_bits()).collect()
}

fn build_replicate<Http, Sleep>(
    harness: &Harness,
    transport: Arc<Http>,
    sleeper: Arc<Sleep>,
    polling: ReplicatePolling,
    policy: OutputPolicy,
) -> Replicate<'_>
where
    Http: ReplicateTransport + 'static,
    Sleep: ReplicateSleeper + 'static,
{
    Replicate {
        transport,
        sleeper,
        prompts: &harness.prompts,
        counter: &harness.counter,
        logger: &harness.logger,
        clock: &harness.clock,
        polling,
        policy,
    }
}

async fn run_complete(
    harness: &Harness,
    responses: Vec<ScriptedResponse>,
    polling: ReplicatePolling,
    policy: OutputPolicy,
    options: &Object,
) -> (
    Result<ReplicateCompletion, ReplicateFailure>,
    Vec<ReplicateRequest>,
    Vec<f64>,
) {
    let recorder = Arc::new(ScriptedTransport::new(
        responses,
        Arc::clone(&harness.events),
    ));
    let sleep_recorder = Arc::new(RecordingSleeper::new());
    let client = build_replicate(
        harness,
        Arc::clone(&recorder),
        Arc::clone(&sleep_recorder),
        polling,
        policy,
    );
    let mut messages = vec![user_message()];
    let mut optional_params = options.clone();
    let defaults = ReplicateDefaults::default();
    let custom_prompts = BTreeMap::new();
    let result = client
        .complete(ReplicateCall {
            model: MODEL,
            messages: &mut messages,
            optional_params: &mut optional_params,
            defaults: &defaults,
            custom_prompts: &custom_prompts,
            api_key: Some("key"),
            now: now(),
        })
        .await;
    (result, recorder.requests(), sleep_recorder.delays())
}

async fn run_stream(
    harness: &Harness,
    responses: Vec<ScriptedResponse>,
    policy: OutputPolicy,
    polling_delay_seconds: f64,
) -> (Vec<ReplicateChunk>, Option<ReplicateFailure>, Vec<f64>) {
    let recorder = Arc::new(ScriptedTransport::new(
        responses,
        Arc::clone(&harness.events),
    ));
    let sleep_recorder = Arc::new(RecordingSleeper::new());
    let mut timing = settings(5, 1.0_f64);
    timing.polling_delay_seconds = polling_delay_seconds;
    let client = build_replicate(
        harness,
        Arc::clone(&recorder),
        Arc::clone(&sleep_recorder),
        timing,
        policy,
    );
    let mut stream = poll_stream(&client, String::from("https://poll/1"), HeaderMap::new());
    let mut chunks = Vec::new();
    let mut failure = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => chunks.push(chunk),
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    (chunks, failure, sleep_recorder.delays())
}

fn chunk(output: &str, status: &str) -> ReplicateChunk {
    ReplicateChunk {
        output: String::from(output),
        status: String::from(status),
    }
}

fn streamed(completion: ReplicateCompletion) -> ReplicateReply {
    let ReplicateCompletion::Reply(reply) = completion else {
        panic!("expected a reply");
    };
    reply
}

#[test]
fn polling_defaults_come_from_the_litellm_constants() {
    let polling = ReplicatePolling::from_env_values(None, None, None, None).unwrap();
    assert_eq!(polling.retries, 5_i32);
    assert_eq!(polling.retry_delay_seconds.to_bits(), 1.0_f64.to_bits());
    assert_eq!(polling.polling_delay_seconds.to_bits(), 0.5_f64.to_bits());
    assert_eq!(polling.model_name_with_id_length, 64);
}

#[test]
fn polling_reads_each_environment_text() {
    let polling =
        ReplicatePolling::from_env_values(Some("2"), Some("4"), Some("0.25"), Some("8")).unwrap();
    assert_eq!(polling.retries, 2_i32);
    assert_eq!(polling.retry_delay_seconds.to_bits(), 4.0_f64.to_bits());
    assert_eq!(polling.polling_delay_seconds.to_bits(), 0.25_f64.to_bits());
    assert_eq!(polling.model_name_with_id_length, 8);
}

#[test]
fn an_empty_environment_text_takes_the_default() {
    let polling =
        ReplicatePolling::from_env_values(Some(""), Some(""), Some(""), Some("")).unwrap();
    assert_eq!(polling.retries, 5_i32);
    assert_eq!(polling.retry_delay_seconds.to_bits(), 1.0_f64.to_bits());
    assert_eq!(polling.polling_delay_seconds.to_bits(), 0.5_f64.to_bits());
    assert_eq!(polling.model_name_with_id_length, 64);
}

#[test]
fn a_bad_environment_text_names_its_variable() {
    let error = ReplicatePolling::from_env_values(Some("five"), None, None, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "environment variable DEFAULT_REPLICATE_POLLING_RETRIES must be a number"
    );
}

#[test]
fn a_fractional_retry_delay_text_is_an_environment_error() {
    let error = ReplicatePolling::from_env_values(None, Some("1.5"), None, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "environment variable DEFAULT_REPLICATE_POLLING_DELAY_SECONDS must be a number"
    );
}

fn assert_create_request(create: &ReplicateRequest) {
    assert_eq!(create.method.as_str(), "POST");
    assert_eq!(
        create.url,
        "https://api.replicate.com/v1/models/meta-llama/llama-2-70b-chat/predictions"
    );
    assert_eq!(create.headers.get("authorization").unwrap(), "Token key");
    assert_eq!(
        serde_json::from_slice::<Object>(create.body.as_deref().unwrap()).unwrap(),
        serde_json::from_str::<Object>(r#"{"input":{"prompt":"<s>[INST] hi [/INST]\n"}}"#).unwrap()
    );
    assert_eq!(create.timeout.as_secs(), 600);
}

fn assert_poll_request(poll: &ReplicateRequest) {
    assert_eq!(poll.method.as_str(), "GET");
    assert_eq!(poll.url, "https://poll/1");
    assert_eq!(poll.headers.get("authorization").unwrap(), "Token key");
    assert_eq!(poll.body, None);
    assert_eq!(poll.timeout.as_secs(), 600);
}

#[tokio::test]
async fn non_stream_polling_waits_the_delay_plus_twice_the_retry() {
    let harness = Harness::new();
    let (result, requests, delays) = run_complete(
        &harness,
        vec![
            reply(200, CREATE),
            reply(200, r#"{"status":"processing"}"#),
            reply(200, r#"{"status":"starting"}"#),
            reply(200, r#"{"status":"succeeded","output":["ok"]}"#),
        ],
        settings(3, 1.0_f64),
        OutputPolicy::Strict,
        &Object::new(),
    )
    .await;
    assert_eq!(
        bit_pattern(&delays),
        [1.0_f64.to_bits(), 3.0_f64.to_bits(), 5.0_f64.to_bits()]
    );
    let reply = streamed(result.unwrap());
    assert_eq!(reply.text, "ok");
    assert_eq!(reply.model, "replicate/meta-llama/llama-2-70b-chat");
    assert_eq!(reply.created, Some(CLOCK_SECONDS));
    assert_eq!(harness.counter.message_models(), [MODEL]);
    assert_eq!(
        harness.counter.responses(),
        [(String::from(MODEL), String::from("ok"))]
    );
    assert_eq!(reply.usage.prompt_tokens, 1);
    assert_eq!(reply.usage.completion_tokens, 2);
    assert_eq!(reply.usage.total_tokens, 3);
    assert_eq!(requests.len(), 4);
    assert_create_request(requests.first().unwrap());
    assert_poll_request(&requests[1]);
}

#[tokio::test]
async fn non_stream_polling_takes_a_non_processing_status_as_the_reply() {
    let harness = Harness::new();
    let (result, _, delays) = run_complete(
        &harness,
        vec![
            reply(200, CREATE),
            reply(200, r#"{"status":"succeeded","output":["ok"]}"#),
        ],
        settings(3, 1.0_f64),
        OutputPolicy::Strict,
        &Object::new(),
    )
    .await;
    assert_eq!(bit_pattern(&delays), [1.0_f64.to_bits()]);
    assert_eq!(streamed(result.unwrap()).text, "ok");
}

#[tokio::test]
async fn a_non_200_poll_reply_is_handed_to_the_reply_transform() {
    let harness = Harness::new();
    let (result, _, delays) = run_complete(
        &harness,
        vec![reply(200, CREATE), reply(500, r#"{"status":"processing"}"#)],
        settings(3, 1.0_f64),
        OutputPolicy::Strict,
        &Object::new(),
    )
    .await;
    assert_eq!(bit_pattern(&delays), [1.0_f64.to_bits()]);
    let Err(error) = result else {
        panic!("expected an error");
    };
    assert_eq!(status(&error), Some(422));
    assert_eq!(
        message(&error),
        Some("LiteLLM Error - prediction not succeeded - {'status': 'processing'}")
    );
}

#[tokio::test]
async fn non_stream_polling_fails_after_the_last_retry() {
    let harness = Harness::new();
    let (result, _, delays) = run_complete(
        &harness,
        vec![
            reply(200, CREATE),
            reply(200, r#"{"status":"processing"}"#),
            reply(200, r#"{"status":"starting"}"#),
        ],
        settings(2, 1.0_f64),
        OutputPolicy::Strict,
        &Object::new(),
    )
    .await;
    assert_eq!(bit_pattern(&delays), [1.0_f64.to_bits(), 3.0_f64.to_bits()]);
    let Err(error) = result else {
        panic!("expected an error");
    };
    assert_eq!(status(&error), Some(500));
    assert_eq!(
        message(&error),
        Some("No response received from Replicate API after max retries")
    );
}

#[rstest]
#[case::strict(OutputPolicy::Strict, Some(CLOCK_SECONDS))]
#[case::lenient(OutputPolicy::Lenient, None)]
#[tokio::test]
async fn created_is_stamped_only_on_the_sync_path(
    #[case] policy: OutputPolicy,
    #[case] expected: Option<i64>,
) {
    let harness = Harness::new();
    let (result, _, _) = run_complete(
        &harness,
        vec![
            reply(200, CREATE),
            reply(200, r#"{"status":"succeeded","output":["ok"]}"#),
        ],
        settings(1, 1.0_f64),
        policy,
        &Object::new(),
    )
    .await;
    assert_eq!(streamed(result.unwrap()).created, expected);
}

#[tokio::test]
async fn the_sync_path_reads_the_clock_right_before_the_create_post() {
    let harness = Harness::new();
    let (result, _, _) = run_complete(
        &harness,
        vec![
            reply(200, CREATE),
            reply(200, r#"{"status":"succeeded","output":["ok"]}"#),
        ],
        settings(1, 1.0_f64),
        OutputPolicy::Strict,
        &Object::new(),
    )
    .await;
    result.unwrap();
    assert_eq!(*harness.events.lock().unwrap(), ["clock", "send", "send"]);
}

#[tokio::test]
async fn a_failing_request_build_reads_no_clock_and_sends_nothing() {
    let harness = Harness::new();
    let mut options = Object::new();
    options.insert(
        String::from("supports_system_prompt"),
        serde_json::from_str(r#""yes""#).unwrap(),
    );
    let (result, requests, _) = run_complete(
        &harness,
        vec![reply(200, CREATE)],
        settings(1, 1.0_f64),
        OutputPolicy::Strict,
        &options,
    )
    .await;
    assert!(matches!(
        result,
        Err(ReplicateFailure::SupportsSystemPrompt)
    ));
    assert!(requests.is_empty());
    assert!(harness.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_async_path_never_reads_the_clock() {
    let harness = Harness::new();
    let (result, _, _) = run_complete(
        &harness,
        vec![
            reply(200, CREATE),
            reply(200, r#"{"status":"succeeded","output":["ok"]}"#),
        ],
        settings(1, 1.0_f64),
        OutputPolicy::Lenient,
        &Object::new(),
    )
    .await;
    result.unwrap();
    assert_eq!(*harness.events.lock().unwrap(), ["send", "send"]);
}

#[rstest]
#[case::strict(OutputPolicy::Strict)]
#[case::lenient(OutputPolicy::Lenient)]
#[tokio::test]
async fn stream_polling_yields_only_the_new_text(#[case] policy: OutputPolicy) {
    let harness = Harness::new();
    let (chunks, failure, delays) = run_stream(
        &harness,
        vec![
            reply(200, r#"{"status":"starting","output":["he"]}"#),
            reply(200, r#"{"status":"processing","output":["hea"]}"#),
            reply(200, r#"{"status":"succeeded","output":["hear"]}"#),
        ],
        policy,
        0.5_f64,
    )
    .await;
    assert_eq!(
        chunks,
        [
            chunk("he", "starting"),
            chunk("a", "processing"),
            chunk("r", "succeeded")
        ]
    );
    assert!(failure.is_none());
    assert_eq!(
        bit_pattern(&delays),
        [0.5_f64.to_bits(), 0.5_f64.to_bits(), 0.5_f64.to_bits()]
    );
}

#[tokio::test]
async fn the_strict_policy_fails_on_a_null_output() {
    let harness = Harness::new();
    let (chunks, failure, _) = run_stream(
        &harness,
        vec![reply(200, r#"{"status":"processing","output":null}"#)],
        OutputPolicy::Strict,
        0.5_f64,
    )
    .await;
    assert!(chunks.is_empty());
    let error = failure.unwrap();
    assert_eq!(status(&error), Some(422));
    assert_eq!(message(&error), Some("Unable to parse response. Got=None"));
    assert_eq!(headers_of(&error), Some(reply_headers()));
}

#[tokio::test]
async fn the_lenient_policy_skips_a_null_output() {
    let harness = Harness::new();
    let (chunks, failure, _) = run_stream(
        &harness,
        vec![
            reply(200, r#"{"status":"processing","output":null}"#),
            reply(200, r#"{"status":"succeeded","output":["z"]}"#),
        ],
        OutputPolicy::Lenient,
        0.5_f64,
    )
    .await;
    assert_eq!(chunks, [chunk("z", "succeeded")]);
    assert!(failure.is_none());
}

#[tokio::test]
async fn the_lenient_policy_renders_a_string_output() {
    let harness = Harness::new();
    let (chunks, failure, _) = run_stream(
        &harness,
        vec![
            reply(200, r#"{"status":"processing","output":"abc"}"#),
            reply(200, r#"{"status":"succeeded","output":"abcd"}"#),
        ],
        OutputPolicy::Lenient,
        0.5_f64,
    )
    .await;
    assert_eq!(
        chunks,
        [chunk("abc", "processing"), chunk("d", "succeeded")]
    );
    assert!(failure.is_none());
}

#[tokio::test]
async fn the_lenient_policy_renders_a_numeric_output() {
    let harness = Harness::new();
    let (chunks, failure, _) = run_stream(
        &harness,
        vec![
            reply(200, r#"{"status":"processing","output":5}"#),
            reply(200, r#"{"status":"succeeded","output":"56"}"#),
        ],
        OutputPolicy::Lenient,
        0.5_f64,
    )
    .await;
    assert_eq!(chunks, [chunk("5", "processing"), chunk("6", "succeeded")]);
    assert!(failure.is_none());
}

#[rstest]
#[case::strict(OutputPolicy::Strict)]
#[case::lenient(OutputPolicy::Lenient)]
#[tokio::test]
async fn a_list_with_a_non_text_item_fails_the_stream(#[case] policy: OutputPolicy) {
    let harness = Harness::new();
    let (chunks, failure, _) = run_stream(
        &harness,
        vec![reply(200, r#"{"status":"processing","output":["a",1]}"#)],
        policy,
        0.5_f64,
    )
    .await;
    assert!(chunks.is_empty());
    let error = failure.unwrap();
    assert_eq!(status(&error), Some(422));
    assert_eq!(
        message(&error),
        Some("Unable to parse response. Got=['a', 1]")
    );
    assert_eq!(headers_of(&error), Some(reply_headers()));
}

#[tokio::test]
async fn a_failed_prediction_yields_its_last_chunk_then_fails() {
    let harness = Harness::new();
    let (chunks, failure, _) = run_stream(
        &harness,
        vec![reply(
            200,
            r#"{"status":"failed","error":"bad","output":["x"]}"#,
        )],
        OutputPolicy::Strict,
        0.5_f64,
    )
    .await;
    assert_eq!(chunks, [chunk("x", "failed")]);
    let error = failure.unwrap();
    assert_eq!(status(&error), Some(400));
    assert_eq!(message(&error), Some("Error: bad"));
    assert_eq!(headers_of(&error), Some(reply_headers()));
}

#[tokio::test]
async fn a_failed_prediction_without_output_fails_at_once() {
    let harness = Harness::new();
    let (chunks, failure, _) = run_stream(
        &harness,
        vec![reply(200, r#"{"status":"failed"}"#)],
        OutputPolicy::Strict,
        0.5_f64,
    )
    .await;
    assert!(chunks.is_empty());
    assert_eq!(message(&failure.unwrap()), Some("Error: "));
}

#[tokio::test]
async fn a_non_200_poll_keeps_polling_without_a_limit() {
    let harness = Harness::new();
    let (chunks, failure, delays) = run_stream(
        &harness,
        vec![
            reply(500, r#"{"status":"processing"}"#),
            reply(200, r#"{"status":"succeeded"}"#),
        ],
        OutputPolicy::Strict,
        0.5_f64,
    )
    .await;
    assert!(chunks.is_empty());
    assert!(failure.is_none());
    assert_eq!(bit_pattern(&delays), [0.5_f64.to_bits(), 0.5_f64.to_bits()]);
}

#[tokio::test]
async fn a_canceled_prediction_ends_the_stream() {
    let harness = Harness::new();
    let (chunks, failure, delays) = run_stream(
        &harness,
        vec![reply(200, r#"{"status":"canceled"}"#)],
        OutputPolicy::Strict,
        0.25_f64,
    )
    .await;
    assert!(chunks.is_empty());
    assert!(failure.is_none());
    assert_eq!(bit_pattern(&delays), [0.25_f64.to_bits()]);
}

#[tokio::test]
async fn a_reply_without_a_status_stops_the_stream() {
    let harness = Harness::new();
    let (_, failure, _) = run_stream(
        &harness,
        vec![reply(200, r#"{"output":["x"]}"#)],
        OutputPolicy::Strict,
        0.5_f64,
    )
    .await;
    assert!(matches!(failure.unwrap(), ReplicateFailure::MissingStatus));
}

#[tokio::test]
async fn complete_returns_a_stream_when_the_option_is_exactly_true() {
    let harness = Harness::new();
    let mut options = Object::new();
    options.insert(
        String::from("stream"),
        serde_json::from_str("true").unwrap(),
    );
    let (result, requests, _) = run_complete(
        &harness,
        vec![reply(200, CREATE)],
        settings(1, 1.0_f64),
        OutputPolicy::Lenient,
        &options,
    )
    .await;
    assert!(matches!(result.unwrap(), ReplicateCompletion::Stream(_)));
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn a_stream_value_that_is_not_exactly_true_stays_non_streaming() {
    let harness = Harness::new();
    let mut options = Object::new();
    options.insert(String::from("stream"), serde_json::from_str("1").unwrap());
    let (result, _, _) = run_complete(
        &harness,
        vec![
            reply(200, CREATE),
            reply(200, r#"{"status":"succeeded","output":["ok"]}"#),
        ],
        settings(1, 1.0_f64),
        OutputPolicy::Strict,
        &options,
    )
    .await;
    assert!(matches!(result.unwrap(), ReplicateCompletion::Reply(_)));
}

fn status(failure: &ReplicateFailure) -> Option<u16> {
    failure.api().map(|error| error.status_code)
}

fn headers_of(failure: &ReplicateFailure) -> Option<HeaderMap> {
    failure.api()?.headers.clone()
}

fn message(failure: &ReplicateFailure) -> Option<&str> {
    failure.api().map(|error| error.message.as_str())
}
