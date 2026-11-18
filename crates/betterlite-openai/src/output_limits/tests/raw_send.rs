use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::super::OUTPUT_TOKEN_LIMIT_MARKER;
use super::super::raw_send::{
    AsyncRawChatTransport, BadRequestReply, RawChatError, RawChatResponse, StatusReply,
    SyncRawChatTransport, send_raw_chat_async, send_raw_chat_sync,
};
use super::{COMPLETION, Recorder, fields, header_map, recovery};

struct ScriptedTransport {
    calls: Mutex<Vec<(Map<String, Value>, f64)>>,
    answers: Mutex<VecDeque<Result<RawChatResponse, RawChatError>>>,
}

impl From<Result<RawChatResponse, RawChatError>> for ScriptedTransport {
    fn from(answer: Result<RawChatResponse, RawChatError>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            answers: Mutex::new(VecDeque::from([answer])),
        }
    }
}

impl ScriptedTransport {
    #[cfg_attr(
        test,
        expect(
            clippy::unwrap_in_result,
            reason = "a poisoned fixture or exhausted script is a test failure, not a provider error"
        )
    )]
    fn recorded(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError> {
        self.calls
            .lock()
            .expect("calls")
            .push((data.clone(), timeout));
        self.answers
            .lock()
            .expect("answers")
            .pop_front()
            .expect("scripted reply")
    }
}

impl SyncRawChatTransport for ScriptedTransport {
    fn create(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError> {
        self.recorded(data, timeout)
    }
}

#[async_trait]
impl AsyncRawChatTransport for ScriptedTransport {
    async fn create(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError> {
        self.recorded(data, timeout)
    }
}

fn fixed_now() -> Duration {
    Duration::ZERO
}

pub(super) async fn azure_raw_send_headers() {
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let recorder = Recorder::new(0);
    let body: Value = serde_json::from_str(COMPLETION).expect("fixture");
    let sync = ScriptedTransport::from(Ok(RawChatResponse {
        headers: header_map(&[("x-ms-request-id", "s1")]),
        body: body.clone(),
    }));
    let sent = send_raw_chat_sync(&sync, &data, 2.5, &recovery(&recorder))
        .await
        .expect("raw answer");
    assert_eq!(sent.headers, header_map(&[("x-ms-request-id", "s1")]));
    assert_eq!(sent.body, body);
    assert_eq!(
        sync.calls.lock().expect("calls").clone(),
        vec![(data.clone(), 2.5_f64)]
    );
    let background = ScriptedTransport::from(Ok(RawChatResponse {
        headers: header_map(&[("x-ms-request-id", "a1")]),
        body: body.clone(),
    }));
    let sent = send_raw_chat_async(&background, &data, 1.5, fixed_now, &recovery(&recorder))
        .await
        .expect("raw answer");
    assert_eq!(sent.headers, header_map(&[("x-ms-request-id", "a1")]));
    assert_eq!(sent.body, body);
    assert_eq!(
        background.calls.lock().expect("calls").clone(),
        vec![(data, 1.5_f64)]
    );
}

pub(super) async fn azure_output_limit_recovery() {
    let recorder = Recorder::new(3);
    let data = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        ("messages", json!([{"role": "user", "content": "hi"}])),
    ]);
    let error = BadRequestReply {
        message: OUTPUT_TOKEN_LIMIT_MARKER.to_uppercase(),
        headers: header_map(&[("x-ms-request-id", "e1")]),
        body: json!({"error": {"message": "limit"}}),
    };
    let sync = ScriptedTransport::from(Err(RawChatError::BadRequest(Box::new(error.clone()))));
    let recovered = send_raw_chat_sync(&sync, &data, 2.5, &recovery(&recorder))
        .await
        .expect("the sync wrapper recovers");
    assert_recovered_completion(&recovered);
    let background =
        ScriptedTransport::from(Err(RawChatError::BadRequest(Box::new(error.clone()))));
    let recovered = send_raw_chat_async(&background, &data, 2.5, fixed_now, &recovery(&recorder))
        .await
        .expect("the async wrapper recovers");
    assert_recovered_completion(&recovered);
    stream_recovers_a_chunk(&recorder, error).await;
    unrelated_failures_propagate(&data, &recorder).await;
}

fn assert_recovered_completion(recovered: &RawChatResponse) {
    assert_eq!(recovered.headers, header_map(&[("x-ms-request-id", "e1")]));
    assert_eq!(
        recovered.body.get("object"),
        Some(&json!("chat.completion"))
    );
    assert_eq!(
        recovered.body.get("id"),
        Some(&json!("chatcmpl-00000000-0000-0000-0000-000000000007"))
    );
    assert_eq!(
        recovered.body.pointer("/usage/prompt_tokens"),
        Some(&json!(3_i32))
    );
}

async fn stream_recovers_a_chunk(recorder: &Recorder, error: BadRequestReply) {
    let streamed = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        ("messages", json!([{"role": "user", "content": "hi"}])),
        ("stream", json!(true)),
    ]);
    let background = ScriptedTransport::from(Err(RawChatError::BadRequest(Box::new(error))));
    let recovered =
        send_raw_chat_async(&background, &streamed, 2.5, fixed_now, &recovery(recorder))
            .await
            .expect("the async wrapper recovers a chunk");
    assert_eq!(
        recovered.body.get("object"),
        Some(&json!("chat.completion.chunk"))
    );
}

fn rate_limited() -> RawChatError {
    RawChatError::Status(Box::new(StatusReply {
        status: 429,
        message: String::from("Rate limit reached for requests"),
        headers: header_map(&[("x-ms-request-id", "e3")]),
        body: json!({"message": "Rate limit reached for requests", "type": "rate_limit_error", "param": null, "code": "rate_limit_exceeded"}),
    }))
}

#[tokio::test]
async fn provider_status_error_keeps_its_response() {
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let recorder = Recorder::new(0);
    let sync = ScriptedTransport::from(Err(rate_limited()));
    let failure = send_raw_chat_sync(&sync, &data, 2.5, &recovery(&recorder))
        .await
        .expect_err("the status error travels on");
    assert_status_failure(&failure);
    let background = ScriptedTransport::from(Err(rate_limited()));
    let failure = send_raw_chat_async(&background, &data, 2.5, fixed_now, &recovery(&recorder))
        .await
        .expect_err("the status error travels on");
    assert_status_failure(&failure);
    assert!(recorder.calls().is_empty());
}

fn assert_status_failure(failure: &RawChatError) {
    let RawChatError::Status(reply) = failure else {
        panic!("a non-400 provider error must propagate: {failure:?}");
    };
    assert_eq!(reply.status, 429);
    assert_eq!(reply.message, "Rate limit reached for requests");
    assert_eq!(reply.headers, header_map(&[("x-ms-request-id", "e3")]));
    assert_eq!(
        reply.body,
        json!({"message": "Rate limit reached for requests", "type": "rate_limit_error", "param": null, "code": "rate_limit_exceeded"})
    );
}

async fn unrelated_failures_propagate(data: &Map<String, Value>, recorder: &Recorder) {
    let unrelated = BadRequestReply {
        message: String::from("invalid temperature"),
        headers: header_map(&[("x-ms-request-id", "e2")]),
        body: json!({"error": {"message": "invalid temperature"}}),
    };
    let sync = ScriptedTransport::from(Err(RawChatError::BadRequest(Box::new(unrelated.clone()))));
    let failure = send_raw_chat_sync(&sync, data, 2.5, &recovery(recorder))
        .await
        .expect_err("an unrelated bad request travels on");
    let RawChatError::BadRequest(rejected) = &failure else {
        panic!("an unrelated bad request must propagate: {failure:?}");
    };
    assert_eq!(rejected.message, "invalid temperature");
    assert_eq!(rejected.headers, header_map(&[("x-ms-request-id", "e2")]));
    assert_eq!(
        rejected.body,
        json!({"error": {"message": "invalid temperature"}})
    );
    let background = ScriptedTransport::from(Err(RawChatError::BadRequest(Box::new(unrelated))));
    assert!(matches!(
        send_raw_chat_async(&background, data, 2.5, fixed_now, &recovery(recorder)).await,
        Err(RawChatError::BadRequest(_))
    ));
    let other = ScriptedTransport::from(Err(RawChatError::Other {
        message: String::from("boom"),
    }));
    assert!(matches!(
        send_raw_chat_sync(&other, data, 2.5, &recovery(recorder)).await,
        Err(RawChatError::Other { message }) if message == "boom"
    ));
}

// the async wrapper reads the wall clock twice, so the fixture advances it on each read
static TICKS: AtomicU64 = AtomicU64::new(0);

fn ticking_now() -> Duration {
    Duration::from_millis(TICKS.fetch_add(1_250, Ordering::SeqCst))
}

pub(super) async fn azure_async_timeout() {
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let recorder = Recorder::new(0);
    TICKS.store(0, Ordering::SeqCst);
    let background = ScriptedTransport::from(Err(RawChatError::Timeout {
        message: String::from("Request timed out."),
    }));
    let failure = send_raw_chat_async(&background, &data, 2.5, ticking_now, &recovery(&recorder))
        .await
        .expect_err("the timeout travels on");
    assert_eq!(
        failure.to_string(),
        "Request timed out. - timeout value=2.5, time taken=1.25 seconds"
    );
    let sync = ScriptedTransport::from(Err(RawChatError::Timeout {
        message: String::from("Request timed out."),
    }));
    let failure = send_raw_chat_sync(&sync, &data, 2.5, &recovery(&recorder))
        .await
        .expect_err("the sync timeout keeps its text");
    assert_eq!(failure.to_string(), "Request timed out.");
    TICKS.store(0, Ordering::SeqCst);
    let background = ScriptedTransport::from(Err(RawChatError::Timeout {
        message: String::from("gone"),
    }));
    let failure = send_raw_chat_async(&background, &data, 0.5, ticking_now, &recovery(&recorder))
        .await
        .expect_err("the second timeout also travels on");
    assert_eq!(
        failure.to_string(),
        "gone - timeout value=0.5, time taken=1.25 seconds"
    );
}

// the documented deviation: an integer-valued timeout prints without Python's trailing .0
#[tokio::test]
async fn integer_timeout_value_prints_without_a_trailing_zero() {
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let recorder = Recorder::new(0);
    TICKS.store(0, Ordering::SeqCst);
    let background = ScriptedTransport::from(Err(RawChatError::Timeout {
        message: String::from("Request timed out."),
    }));
    let failure = send_raw_chat_async(&background, &data, 2.0, ticking_now, &recovery(&recorder))
        .await
        .expect_err("the timeout travels on");
    assert_eq!(
        failure.to_string(),
        "Request timed out. - timeout value=2, time taken=1.25 seconds"
    );
}

pub(super) async fn unrelated_error() {
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let recorder = Recorder::new(7);
    let body = json!({"message": "could not finish the message", "type": "invalid_request_error", "param": "max_tokens", "code": null});
    let transport =
        ScriptedTransport::from(Err(RawChatError::BadRequest(Box::new(BadRequestReply {
            message: String::from("could not finish the message"),
            headers: header_map(&[("x-request-id", "r1")]),
            body: body.clone(),
        }))));
    let sync = send_raw_chat_sync(&transport, &data, 2.5, &recovery(&recorder)).await;
    transport
        .answers
        .lock()
        .expect("answers")
        .push_back(Err(RawChatError::BadRequest(Box::new(BadRequestReply {
            message: String::from("could not finish the message"),
            headers: header_map(&[("x-request-id", "r1")]),
            body: body.clone(),
        }))));
    let asynchronous =
        send_raw_chat_async(&transport, &data, 2.5, fixed_now, &recovery(&recorder)).await;
    for answer in [sync, asynchronous] {
        let Err(RawChatError::BadRequest(error)) = answer else {
            panic!("an unrelated bad request must propagate");
        };
        assert_eq!(error.body, body);
        assert_eq!(error.message, "could not finish the message");
        assert_eq!(error.headers, header_map(&[("x-request-id", "r1")]));
    }
    assert_eq!(
        *transport.calls.lock().expect("calls"),
        vec![(data.clone(), 2.5_f64), (data, 2.5_f64)]
    );
    assert!(recorder.calls().is_empty());
}

mod errors;
