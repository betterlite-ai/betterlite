use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use http::HeaderMap;
use serde_json::{Map, Value, json};

use super::super::OUTPUT_TOKEN_LIMIT_MARKER;
use super::super::raw_send::{
    AsyncRawChatTransport, BadRequestReply, RawChatError, RawChatResponse, SyncRawChatTransport,
    send_raw_chat_async, send_raw_chat_sync,
};
use super::{COMPLETION, Recorder, fields, header_map, recovery};

struct ScriptedTransport {
    calls: Mutex<Vec<(Map<String, Value>, f64)>>,
    answer: Result<RawChatResponse, RawChatError>,
}

impl ScriptedTransport {
    fn recorded(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError> {
        self.calls
            .lock()
            .expect("calls")
            .push((data.clone(), timeout));
        self.answer.clone()
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

fn raw_answer(headers: HeaderMap, body: Value) -> Result<RawChatResponse, RawChatError> {
    Ok(RawChatResponse { headers, body })
}

pub(super) async fn azure_raw_send_headers() {
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let recorder = Recorder::new(0);
    let body: Value = serde_json::from_str(COMPLETION).expect("fixture");
    let sync = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: raw_answer(header_map(&[("x-ms-request-id", "s1")]), body.clone()),
    };
    let sent = send_raw_chat_sync(&sync, &data, 2.5, &recovery(&recorder))
        .await
        .expect("raw answer");
    assert_eq!(sent.headers, header_map(&[("x-ms-request-id", "s1")]));
    assert_eq!(sent.body, body);
    assert_eq!(
        sync.calls.lock().expect("calls").clone(),
        vec![(data.clone(), 2.5)]
    );
    let background = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: raw_answer(header_map(&[("x-ms-request-id", "a1")]), body.clone()),
    };
    let sent = send_raw_chat_async(&background, &data, 1.5, fixed_now, &recovery(&recorder))
        .await
        .expect("raw answer");
    assert_eq!(sent.headers, header_map(&[("x-ms-request-id", "a1")]));
    assert_eq!(sent.body, body);
    assert_eq!(
        background.calls.lock().expect("calls").clone(),
        vec![(data, 1.5)]
    );
}

pub(super) async fn azure_output_limit_recovery() {
    let recorder = Recorder::new(3);
    let data = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        ("messages", json!([{"role": "user", "content": "hi"}])),
    ]);
    let error = RawChatError::BadRequest(Box::new(BadRequestReply {
        message: OUTPUT_TOKEN_LIMIT_MARKER.to_uppercase(),
        headers: header_map(&[("x-ms-request-id", "e1")]),
        body: json!({"error": {"message": "limit"}}),
    }));
    let sync = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(error.clone()),
    };
    let recovered = send_raw_chat_sync(&sync, &data, 2.5, &recovery(&recorder))
        .await
        .expect("the sync wrapper recovers");
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
        Some(&json!(3))
    );
    let background = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(error.clone()),
    };
    let recovered = send_raw_chat_async(&background, &data, 2.5, fixed_now, &recovery(&recorder))
        .await
        .expect("the async wrapper recovers");
    assert_eq!(recovered.headers, header_map(&[("x-ms-request-id", "e1")]));
    assert_eq!(
        recovered.body.get("object"),
        Some(&json!("chat.completion"))
    );
    let streamed = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        ("messages", json!([{"role": "user", "content": "hi"}])),
        ("stream", json!(true)),
    ]);
    let background = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(error),
    };
    let recovered =
        send_raw_chat_async(&background, &streamed, 2.5, fixed_now, &recovery(&recorder))
            .await
            .expect("the async wrapper recovers a chunk");
    assert_eq!(
        recovered.body.get("object"),
        Some(&json!("chat.completion.chunk"))
    );
    let unrelated = RawChatError::BadRequest(Box::new(BadRequestReply {
        message: String::from("invalid temperature"),
        headers: header_map(&[("x-ms-request-id", "e2")]),
        body: json!({"error": {"message": "invalid temperature"}}),
    }));
    let sync = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(unrelated.clone()),
    };
    let failure = send_raw_chat_sync(&sync, &data, 2.5, &recovery(&recorder))
        .await
        .expect_err("an unrelated bad request travels on");
    match &failure {
        RawChatError::BadRequest(rejected) => {
            assert_eq!(rejected.message, "invalid temperature");
            assert_eq!(rejected.headers, header_map(&[("x-ms-request-id", "e2")]));
            assert_eq!(
                rejected.body,
                json!({"error": {"message": "invalid temperature"}})
            );
        }
        other => panic!("{other:?}"),
    }
    let background = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(unrelated),
    };
    assert!(matches!(
        send_raw_chat_async(&background, &data, 2.5, fixed_now, &recovery(&recorder)).await,
        Err(RawChatError::BadRequest(_))
    ));
    let other = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(RawChatError::Other {
            message: String::from("boom"),
        }),
    };
    assert!(matches!(
        send_raw_chat_sync(&other, &data, 2.5, &recovery(&recorder)).await,
        Err(RawChatError::Other { message }) if message == "boom"
    ));
}

// the async wrapper reads the wall clock twice, so the fixture advances it on each read
static TICKS: AtomicU64 = AtomicU64::new(0);

fn ticking_now() -> Duration {
    Duration::from_millis(TICKS.fetch_add(1_250, Ordering::SeqCst))
}

fn failure_text(error: &RawChatError) -> String {
    match error {
        RawChatError::Timeout { message } | RawChatError::Other { message } => message.clone(),
        RawChatError::BadRequest(rejected) => rejected.message.clone(),
    }
}

pub(super) async fn azure_async_timeout() {
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let recorder = Recorder::new(0);
    TICKS.store(0, Ordering::SeqCst);
    let background = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(RawChatError::Timeout {
            message: String::from("Request timed out."),
        }),
    };
    let failure = send_raw_chat_async(&background, &data, 2.5, ticking_now, &recovery(&recorder))
        .await
        .expect_err("the timeout travels on");
    assert_eq!(
        failure_text(&failure),
        "Request timed out. - timeout value=2.5, time taken=1.25 seconds"
    );
    let sync = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(RawChatError::Timeout {
            message: String::from("Request timed out."),
        }),
    };
    let failure = send_raw_chat_sync(&sync, &data, 2.5, &recovery(&recorder))
        .await
        .expect_err("the sync timeout keeps its text");
    assert_eq!(failure_text(&failure), "Request timed out.");
    TICKS.store(0, Ordering::SeqCst);
    let background = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(RawChatError::Timeout {
            message: String::from("gone"),
        }),
    };
    let failure = send_raw_chat_async(&background, &data, 0.5, ticking_now, &recovery(&recorder))
        .await
        .expect_err("the second timeout also travels on");
    assert_eq!(
        failure_text(&failure),
        "gone - timeout value=0.5, time taken=1.25 seconds"
    );
}

pub(super) async fn unrelated_error() {
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let recorder = Recorder::new(7);
    let body = json!({"message": "could not finish the message", "type": "invalid_request_error", "param": "max_tokens", "code": null});
    let transport = ScriptedTransport {
        calls: Mutex::new(Vec::new()),
        answer: Err(RawChatError::BadRequest(Box::new(BadRequestReply {
            message: String::from("could not finish the message"),
            headers: header_map(&[("x-request-id", "r1")]),
            body: body.clone(),
        }))),
    };
    let sync = send_raw_chat_sync(&transport, &data, 2.5, &recovery(&recorder)).await;
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
        vec![(data.clone(), 2.5), (data, 2.5)]
    );
    assert!(recorder.calls().is_empty());
}
