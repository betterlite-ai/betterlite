#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, pending, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::time::Instant;
use url::Url;

use super::error::EditCallError;
use super::prepare::PreparedEdit;
use super::{EditCall, EditExecutor, LogContext, prepare_call};
use crate::api::bfl::edit::media::{MaskInput, SuppliedMedia};
use crate::api::bfl::generation::BflImageResponse;
use crate::api::bfl::generation_call::params::CallParams;
use crate::api::bfl::generation_call::submit::{SubmitError, SubmitRequest, SubmitTransport};
use crate::api::bfl::poll::{MonotonicClock, PollError, PollTransport, TokioSleeper};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

const MODEL: &str = "black_forest_labs/flux-kontext-pro";
const PROMPT: &str = "make it blue";
const BASE: &str = "https://api.bfl.ai";
const NOW: i64 = 1_700_000_000;
const IMAGE: &[u8] = b"first image";
const SECOND: &[u8] = b"second image";
const SUBMITTED: &str =
    r#"{"id":"job-1","polling_url":"https://gateway.bfl.ai/v1/get_result?id=1"}"#;
const PENDING: &str = r#"{"status":"Pending"}"#;
const READY: &str = r#"{"status":"Ready","result":{"sample":"https://delivery.test/edited.png"}}"#;

#[derive(Clone, Copy)]
struct Reply {
    status: u16,
    body: &'static str,
}

struct Wire {
    submission: Reply,
    polls: Mutex<VecDeque<Reply>>,
    posts: Mutex<Vec<SubmitRequest>>,
    gets: Mutex<Vec<(String, HeaderMap)>>,
    hang_poll: AtomicBool,
    fail_post: AtomicBool,
    fail_get: AtomicBool,
    cancelled: Arc<AtomicBool>,
}

impl Wire {
    fn new(submission: Reply, polls: Vec<Reply>) -> Self {
        Self {
            submission,
            polls: Mutex::new(VecDeque::from(polls)),
            posts: Mutex::default(),
            gets: Mutex::default(),
            hang_poll: AtomicBool::new(false),
            fail_post: AtomicBool::new(false),
            fail_get: AtomicBool::new(false),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    fn ready() -> Self {
        Self::new(
            Reply {
                status: 200,
                body: SUBMITTED,
            },
            vec![Reply {
                status: 200,
                body: READY,
            }],
        )
    }

    fn posts(&self) -> MutexGuard<'_, Vec<SubmitRequest>> {
        self.posts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn gets(&self) -> MutexGuard<'_, Vec<(String, HeaderMap)>> {
        self.gets.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn posted_body(&self) -> Json {
        self.posts()[0].body.clone()
    }

    fn upstream(reply: Reply) -> UpstreamResponse {
        UpstreamResponse {
            status: reply.status,
            headers: HeaderMap::new(),
            body: reply.body.as_bytes().to_vec(),
        }
    }
}

struct Cancel(Arc<AtomicBool>);

impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl SubmitTransport for Wire {
    fn post(
        &self,
        request: SubmitRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, SubmitError>> + Send {
        self.posts().push(request);
        let posted = if self.fail_post.load(Ordering::SeqCst) {
            Err(SubmitError::Transport(TransportError::Closed))
        } else {
            Ok(Self::upstream(self.submission))
        };
        ready(posted)
    }
}

impl PollTransport for Wire {
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        self.gets().push((url.to_string(), headers));
        if self.fail_get.load(Ordering::SeqCst) {
            return Err(TransportError::Closed);
        }
        if self.hang_poll.load(Ordering::SeqCst) {
            let _held = Cancel(Arc::clone(&self.cancelled));
            return pending::<Result<UpstreamResponse, TransportError>>().await;
        }
        let reply = {
            let mut polls = self.polls.lock().unwrap_or_else(PoisonError::into_inner);
            polls.pop_front().unwrap_or(self.submission)
        };
        Ok(Self::upstream(reply))
    }
}

#[derive(Default)]
struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"flux-kontext-pro","call_type":"image_edit","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
    }

    fn prepare(&mut self, call: &mut EditCall<'_>) -> Result<PreparedEdit, EditCallError> {
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        };
        prepare_call(
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
    }

    fn details(&self) -> IndexMap<String, Json> {
        self.state.model_call_details.clone()
    }
}

struct Outcome {
    result: Result<BflImageResponse, EditCallError>,
    details: IndexMap<String, Json>,
}

fn edit_call<'borrow>(
    image: &'borrow mut SuppliedMedia,
    optional_params: &'borrow ImageParams,
    extra_headers: &'borrow [(String, String)],
    prompt: Option<&'borrow str>,
) -> EditCall<'borrow> {
    EditCall {
        model: MODEL,
        prompt,
        image: Some(image),
        mask: MaskInput::Absent,
        optional_params,
        params: CallParams {
            api_key: Some(SecretString::from(String::from("sk-bfl"))),
            api_base: Some(String::from(BASE)),
        },
        extra_headers,
        timeout: None,
        aimage_edit: false,
        max_depth: 100,
        configured_key: None,
        configured_base: BASE,
        now_seconds: NOW,
    }
}

async fn run(wire: &Wire, call: &mut EditCall<'_>) -> Outcome {
    let mut fixture = Fixture::new();
    let preparation = fixture.prepare(call);
    let clock = MonotonicClock {
        origin: Instant::now(),
    };
    let sleeper = TokioSleeper;
    let executor = EditExecutor {
        provider: wire,
        clock: &clock,
        sleeper: &sleeper,
    };
    let supplied: Option<SuppliedClient<'_, Wire>> = None;
    let result = match preparation {
        Ok(prepared) => executor.image_edit(call, &prepared, supplied).await,
        Err(error) => Err(error),
    };
    Outcome {
        result,
        details: fixture.details(),
    }
}

fn text(value: Option<&Json>) -> &str {
    match value {
        Some(Json::Text(found)) => found,
        _ => panic!("the fixture field is text"),
    }
}

fn object(value: Option<&Json>) -> &IndexMap<String, Json> {
    match value {
        Some(Json::Object(fields)) => fields,
        _ => panic!("the fixture field is an object"),
    }
}

fn field<'json>(body: &'json Json, name: &str) -> &'json Json {
    match body {
        Json::Object(fields) => fields.get(name).expect("the body names the field"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the submission body is an object")
        }
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn poll_once<F>(future: Pin<&mut F>) -> Poll<F::Output>
where
    F: Future,
{
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    future.poll(&mut context)
}

#[rstest]
#[case(Some(PROMPT), PROMPT, Json::Text(String::from(PROMPT)))]
#[case(None, "", Json::Null)]
#[tokio::test]
async fn the_prompt_reaches_the_request_normalized_and_the_record_as_given(
    #[case] prompt: Option<&str>,
    #[case] sent: &str,
    #[case] recorded: Json,
) {
    let wire = Wire::ready();
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], prompt);

    let outcome = run(&wire, &mut call).await;

    outcome.result.expect("the edit succeeds");
    assert_eq!(text(Some(field(&wire.posted_body(), "prompt"))), sent);
    assert_eq!(outcome.details.get("input"), Some(&recorded));
    assert_eq!(text(outcome.details.get("log_event_type")), "pre_api_call");
    let additional = object(outcome.details.get("additional_args"));
    assert_eq!(
        text(additional.get("api_base")),
        "https://api.bfl.ai/v1/flux-kontext-pro"
    );
    let body = object(additional.get("complete_input_dict"));
    assert_eq!(text(body.get("input_image")), BASE64.encode(IMAGE));
    let headers = object(additional.get("headers"));
    assert_eq!(text(headers.get("content-type")), "application/json");
    assert_eq!(text(headers.get("accept")), "application/json");
}

#[tokio::test]
async fn the_first_member_of_an_image_list_is_the_submitted_image() {
    let wire = Wire::ready();
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::List(vec![
        SuppliedMedia::Bytes(IMAGE.to_vec()),
        SuppliedMedia::Bytes(SECOND.to_vec()),
    ]);
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    outcome.result.expect("the edit succeeds");
    assert_eq!(
        text(Some(field(&wire.posted_body(), "input_image"))),
        BASE64.encode(IMAGE)
    );
}

#[tokio::test]
async fn an_empty_image_list_is_a_400_before_the_record_or_the_post() {
    let wire = Wire::ready();
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::List(Vec::new());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome.result.expect_err("an empty image list fails");
    assert!(matches!(error, EditCallError::NoImageProvided));
    assert_eq!(error.to_string(), "No image provided");
    assert_eq!(error.status(), Some(400));
    assert!(outcome.details.get("log_event_type").is_none());
    assert!(wire.posts().is_empty());
    assert!(wire.gets().is_empty());
}

#[tokio::test]
async fn the_optional_extra_headers_seed_the_headers_the_credential_overwrites() {
    let wire = Wire::ready();
    let mut optional_params = ImageParams::new();
    let inner = IndexMap::from([
        (String::from("x-key"), Json::Text(String::from("inner-key"))),
        (
            String::from("Content-Type"),
            Json::Text(String::from("text/inner")),
        ),
        (String::from("X-Inner"), Json::Text(String::from("inner"))),
    ]);
    optional_params.insert(String::from("extra_headers"), Json::Object(inner));
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    outcome.result.expect("the edit succeeds");
    let posts = wire.posts();
    assert_eq!(
        header(&posts[0].headers, "x-key").as_deref(),
        Some("sk-bfl")
    );
    assert_eq!(
        header(&posts[0].headers, "content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        header(&posts[0].headers, "x-inner").as_deref(),
        Some("inner")
    );
    let gets = wire.gets();
    assert_eq!(header(&gets[0].1, "x-key").as_deref(), Some("sk-bfl"));
    assert_eq!(gets[0].1.len(), 1, "the poll carries just x-key");
    let headers = object(object(outcome.details.get("additional_args")).get("headers"));
    assert_eq!(text(headers.get("x-inner")), "inner");
}

#[rstest]
#[case("x-key")]
#[case("X-Key")]
#[tokio::test]
async fn the_caller_extra_headers_overlay_the_validated_ones_in_the_post_and_the_poll(
    #[case] key_name: &str,
) {
    let wire = Wire::ready();
    let optional_params = ImageParams::new();
    let extra = [
        (String::from(key_name), String::from("caller-key")),
        (String::from("X-Trace"), String::from("t-1")),
    ];
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &extra, Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    outcome.result.expect("the edit succeeds");
    let posts = wire.posts();
    assert_eq!(
        header(&posts[0].headers, "x-key").as_deref(),
        Some("caller-key")
    );
    assert_eq!(header(&posts[0].headers, "x-trace").as_deref(), Some("t-1"));
    assert_eq!(
        posts[0].headers.get_all("x-key").iter().count(),
        1,
        "the case-colliding name replaces the validated header"
    );
    let gets = wire.gets();
    assert_eq!(header(&gets[0].1, "x-key").as_deref(), Some("caller-key"));
    let headers = object(object(outcome.details.get("additional_args")).get("headers"));
    assert_eq!(text(headers.get("x-trace")), "t-1");
}

#[rstest]
#[case("bad name", "value", "name")]
#[case("x-trace", "bad\nvalue", "value")]
#[case("x-trace", "trace-\u{e9}", "ascii")]
#[tokio::test]
async fn a_caller_header_the_wire_cannot_carry_is_a_500_after_the_record(
    #[case] name: &str,
    #[case] value: &str,
    #[case] refused: &str,
) {
    let wire = Wire::ready();
    let optional_params = ImageParams::new();
    let extra = [(String::from(name), String::from(value))];
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &extra, Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome
        .result
        .expect_err("the header cannot reach the wire");
    let found = match &error {
        EditCallError::HeaderName { .. } => "name",
        EditCallError::HeaderValue { .. } => "value",
        EditCallError::HeaderNotAscii { .. } => "ascii",
        EditCallError::Bfl(_)
        | EditCallError::Edit(_)
        | EditCallError::Params(_)
        | EditCallError::Logging(_)
        | EditCallError::NoImageProvided
        | EditCallError::InnerHeadersType
        | EditCallError::InnerHeaderValue { .. }
        | EditCallError::Submission { .. }
        | EditCallError::SubmissionStatus { .. }
        | EditCallError::Poll(_)
        | EditCallError::MissingCompletedResponse => {
            panic!("the header is refused by its own check")
        }
    };
    assert_eq!(found, refused);
    assert_eq!(error.status(), Some(500));
    let headers = object(object(outcome.details.get("additional_args")).get("headers"));
    assert_eq!(text(headers.get(&name.to_ascii_lowercase())), value);
    assert!(wire.posts().is_empty());
}

#[rstest]
#[case("outer", "must be an object")]
#[case("value", "extra_headers.x-inner must be a string")]
#[tokio::test]
async fn optional_extra_headers_of_the_wrong_shape_are_named(
    #[case] shape: &str,
    #[case] message: &str,
) {
    let wire = Wire::ready();
    let mut optional_params = ImageParams::new();
    let inner = if shape == "outer" {
        Json::Text(String::from("not a map"))
    } else {
        Json::Object(IndexMap::from([(
            String::from("x-inner"),
            Json::List(Vec::new()),
        )]))
    };
    optional_params.insert(String::from("extra_headers"), inner);
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome
        .result
        .expect_err("a wrong-shaped extra_headers fails");
    assert!(error.to_string().contains(message));
    assert_eq!(error.status(), None);
    assert!(wire.posts().is_empty());
}

#[tokio::test]
async fn an_inner_header_the_wire_cannot_carry_is_a_500_before_the_record() {
    let wire = Wire::ready();
    let mut optional_params = ImageParams::new();
    let inner = IndexMap::from([(String::from("bad name"), Json::Text(String::from("inner")))]);
    optional_params.insert(String::from("extra_headers"), Json::Object(inner));
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome
        .result
        .expect_err("the seeded header cannot reach the wire");
    assert!(matches!(error, EditCallError::HeaderName { .. }));
    assert_eq!(error.status(), Some(500));
    assert!(outcome.details.get("log_event_type").is_none());
    assert!(wire.posts().is_empty());
}

#[tokio::test]
async fn the_submission_carries_the_requested_timeout() {
    let wire = Wire::ready();
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));
    call.timeout = Some(Duration::from_millis(12_500));

    let outcome = run(&wire, &mut call).await;

    outcome.result.expect("the edit succeeds");
    assert_eq!(wire.posts()[0].timeout, Some(Duration::from_millis(12_500)));
    assert_eq!(
        wire.posts()[0].url.as_str(),
        "https://api.bfl.ai/v1/flux-kontext-pro"
    );
}

#[rstest]
#[case(200)]
#[case(299)]
#[tokio::test]
async fn a_submission_status_from_200_through_299_is_polled(#[case] status: u16) {
    let wire = Wire::new(
        Reply {
            status,
            body: SUBMITTED,
        },
        vec![Reply {
            status: 200,
            body: READY,
        }],
    );
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    outcome.result.expect("a 2xx submission is polled");
    assert_eq!(wire.gets().len(), 1);
}

#[rstest]
#[case(199)]
#[case(300)]
#[case(400)]
#[tokio::test]
async fn a_submission_outside_2xx_is_a_500_without_a_poll(#[case] status: u16) {
    let wire = Wire::new(
        Reply {
            status,
            body: SUBMITTED,
        },
        vec![Reply {
            status: 200,
            body: READY,
        }],
    );
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome.result.expect_err("a non-2xx submission fails");
    assert!(matches!(
        &error,
        EditCallError::SubmissionStatus { status: found } if *found == status
    ));
    assert_eq!(error.status(), Some(500));
    assert!(wire.gets().is_empty());
}

#[tokio::test]
async fn a_failed_submission_post_is_a_500_after_the_pre_call_record() {
    let wire = Wire::ready();
    wire.fail_post.store(true, Ordering::SeqCst);
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome.result.expect_err("a failed submission fails");
    assert!(matches!(
        &error,
        EditCallError::Submission {
            source: SubmitError::Transport(TransportError::Closed)
        }
    ));
    assert_eq!(error.status(), Some(500));
    assert_eq!(text(outcome.details.get("input")), PROMPT);
    assert_eq!(wire.posts().len(), 1);
    assert!(wire.gets().is_empty());
}

#[tokio::test]
async fn missing_credentials_fail_before_a_submission() {
    let wire = Wire::ready();
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));
    call.params = CallParams {
        api_key: None,
        api_base: None,
    };

    let outcome = run(&wire, &mut call).await;

    let error = outcome.result.expect_err("an absent key fails");
    assert_eq!(error.status(), Some(401));
    assert!(wire.posts().is_empty());
}

#[tokio::test]
async fn the_completed_reply_is_projected_into_the_single_edited_image() {
    let wire = Wire::ready();
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let response = outcome.result.expect("the edit succeeds");
    assert_eq!(response.created, NOW);
    assert_eq!(response.data.len(), 1);
    assert_eq!(
        response.data[0].url.as_deref(),
        Some("https://delivery.test/edited.png")
    );
    assert_eq!(wire.posts().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_pending_poll_asks_again_before_the_projection() {
    let wire = Wire::new(
        Reply {
            status: 200,
            body: SUBMITTED,
        },
        vec![
            Reply {
                status: 200,
                body: PENDING,
            },
            Reply {
                status: 200,
                body: READY,
            },
        ],
    );
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    outcome.result.expect("the edit succeeds");
    assert_eq!(wire.gets().len(), 2);
    assert_eq!(wire.posts().len(), 1, "polling sends no second submission");
    assert_eq!(
        wire.gets()[0].0,
        "https://gateway.bfl.ai/v1/get_result?id=1"
    );
}

#[tokio::test]
async fn a_failed_poll_is_a_400_with_the_generation_message() {
    let wire = Wire::new(
        Reply {
            status: 200,
            body: SUBMITTED,
        },
        vec![Reply {
            status: 200,
            body: r#"{"status":"Content Moderated"}"#,
        }],
    );
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome.result.expect_err("a moderated edit fails");
    assert_eq!(error.status(), Some(400));
    assert_eq!(
        error.to_string(),
        "Image generation failed: Content Moderated"
    );
    assert_eq!(wire.posts().len(), 1);
    assert_eq!(wire.gets().len(), 1);
}

#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn a_poll_failure_without_a_status_surfaces_it_unset(#[case] broken_transport: bool) {
    let body = if broken_transport {
        SUBMITTED
    } else {
        "not a task"
    };
    let wire = Wire::new(
        Reply {
            status: 200,
            body: SUBMITTED,
        },
        vec![Reply { status: 200, body }],
    );
    wire.fail_get.store(broken_transport, Ordering::SeqCst);
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome.result.expect_err("a broken poll fails");
    assert!(matches!(
        &error,
        EditCallError::Poll(PollError::PollDecode { .. } | PollError::PollTransport { .. })
    ));
    assert_eq!(error.status(), None);
    assert_eq!(
        wire.posts().len(),
        1,
        "the poll failure follows the submission"
    );
}

#[tokio::test]
async fn a_ready_reply_without_a_sample_is_a_500() {
    let wire = Wire::new(
        Reply {
            status: 200,
            body: SUBMITTED,
        },
        vec![Reply {
            status: 200,
            body: r#"{"status":"Ready","result":{}}"#,
        }],
    );
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));

    let outcome = run(&wire, &mut call).await;

    let error = outcome.result.expect_err("a result without a URL fails");
    assert_eq!(error.status(), Some(500));
}

#[rstest]
#[case(false, ClientKind::Sync, true)]
#[case(true, ClientKind::Asynchronous, true)]
#[case(false, ClientKind::Asynchronous, false)]
#[case(true, ClientKind::Sync, false)]
#[tokio::test]
async fn only_a_supplied_transport_of_the_entry_points_kind_is_kept(
    #[case] async_call: bool,
    #[case] kind: ClientKind,
    #[case] kept: bool,
) {
    let provider = Wire::ready();
    let supplied = Wire::ready();
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));
    call.aimage_edit = async_call;
    let mut fixture = Fixture::new();
    let prepared = fixture.prepare(&mut call).expect("the call is prepared");
    let clock = MonotonicClock {
        origin: Instant::now(),
    };
    let sleeper = TokioSleeper;
    let executor = EditExecutor {
        provider: &provider,
        clock: &clock,
        sleeper: &sleeper,
    };
    let handed = SuppliedClient {
        kind,
        transport: &supplied,
    };

    let result = executor.image_edit(&call, &prepared, Some(handed)).await;

    result.expect("the edit succeeds");
    assert_eq!(supplied.posts().len(), usize::from(kept));
    assert_eq!(provider.posts().len(), usize::from(!kept));
}

#[tokio::test]
async fn dropping_a_pending_edit_poll_cancels_the_poll() {
    let wire = Wire::ready();
    wire.hang_poll.store(true, Ordering::SeqCst);
    let optional_params = ImageParams::new();
    let mut image = SuppliedMedia::Bytes(IMAGE.to_vec());
    let mut call = edit_call(&mut image, &optional_params, &[], Some(PROMPT));
    let mut fixture = Fixture::new();
    let prepared = fixture.prepare(&mut call).expect("the call is prepared");
    let clock = MonotonicClock {
        origin: Instant::now(),
    };
    let sleeper = TokioSleeper;
    let executor = EditExecutor {
        provider: &wire,
        clock: &clock,
        sleeper: &sleeper,
    };
    let supplied: Option<SuppliedClient<'_, Wire>> = None;

    let mut future = Box::pin(executor.image_edit(&call, &prepared, supplied));
    assert!(poll_once(future.as_mut()).is_pending());
    drop(future);

    assert!(wire.cancelled.load(Ordering::SeqCst));
}
