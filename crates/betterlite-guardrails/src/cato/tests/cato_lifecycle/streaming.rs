use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::websocket_client::error::WebSocketClientError;
use reqwest::header::HeaderMap;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time;

use super::ws_guardrail;
use crate::cato::CatoGuardrail;
use crate::cato::error::{CatoError, ModelStreamError};
use crate::cato::tests::{list, object, text};
use crate::cato::transport::stream::{
    CONNECT_TIMEOUT, CatoFrameReader, CatoFrameWriter, CatoSocket, CatoSocketConnector, DONE_FRAME,
    MAX_FRAME_BYTES, relay_output_stream,
};

type Sent = Arc<Mutex<Vec<String>>>;
type Chunks = mpsc::Receiver<Result<GuardrailValue, ModelStreamError>>;

// a relay that stops consuming or waiting would otherwise hold the test suite until its timeout
const RELAY_DEADLINE: Duration = Duration::from_secs(5);

enum FakeFrame {
    Message(Result<Option<Vec<u8>>, CatoError>),
    AfterSentTexts(usize, Result<Option<Vec<u8>>, CatoError>),
}

struct FakeReader {
    frames: Arc<Mutex<VecDeque<FakeFrame>>>,
    sent: Sent,
    sent_signal: Arc<Notify>,
}

#[async_trait]
impl CatoFrameReader for FakeReader {
    // a frame leaves the queue only once it can be returned, so a cancelled receive loses nothing
    async fn recv_message(&mut self) -> Result<Option<Vec<u8>>, CatoError> {
        loop {
            let frame = {
                let mut frames = self.frames.lock().expect("frames");
                let poppable = match frames.front() {
                    Some(FakeFrame::AfterSentTexts(count, _)) => {
                        self.sent.lock().expect("sent").len() >= *count
                    }
                    Some(FakeFrame::Message(_)) => true,
                    None => false,
                };
                if poppable { frames.pop_front() } else { None }
            };
            match frame {
                Some(FakeFrame::Message(answer) | FakeFrame::AfterSentTexts(_, answer)) => {
                    return answer;
                }
                None => self.sent_signal.notified().await,
            }
        }
    }
}

struct FakeWriter {
    sent: Sent,
    sent_signal: Arc<Notify>,
    fail_writes: bool,
    // dropping the writer closes this channel, which tells the test the sender task is gone
    _closed: Option<oneshot::Sender<()>>,
}

#[async_trait]
impl CatoFrameWriter for FakeWriter {
    async fn send_text(&mut self, text: &str) -> Result<(), CatoError> {
        if self.fail_writes {
            return Err(CatoError::WebSocket {
                source: WebSocketClientError::Protocol,
            });
        }
        self.sent.lock().expect("sent").push(text.to_owned());
        self.sent_signal.notify_one();
        Ok(())
    }
}

struct ConnectCall {
    url: String,
    open: Duration,
    read: Duration,
}

struct FakeConnector {
    frames: Arc<Mutex<VecDeque<FakeFrame>>>,
    sent: Sent,
    sent_signal: Arc<Notify>,
    connect_call: Arc<Mutex<Option<ConnectCall>>>,
    closed: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    fail_writes: bool,
}

struct FakeControl {
    sent: Sent,
    connect_call: Arc<Mutex<Option<ConnectCall>>>,
    closed: oneshot::Receiver<()>,
}

#[async_trait]
impl CatoSocketConnector for FakeConnector {
    async fn connect(
        &self,
        url: &str,
        _headers: &HeaderMap,
        open: Duration,
        read: Duration,
    ) -> Result<CatoSocket, CatoError> {
        *self.connect_call.lock().expect("connect") = Some(ConnectCall {
            url: url.to_owned(),
            open,
            read,
        });
        Ok(CatoSocket {
            reader: Box::new(FakeReader {
                frames: Arc::clone(&self.frames),
                sent: Arc::clone(&self.sent),
                sent_signal: Arc::clone(&self.sent_signal),
            }),
            writer: Box::new(FakeWriter {
                sent: Arc::clone(&self.sent),
                sent_signal: Arc::clone(&self.sent_signal),
                fail_writes: self.fail_writes,
                _closed: self.closed.lock().expect("closed").take(),
            }),
        })
    }
}

fn fake_connector(frames: Vec<FakeFrame>, fail_writes: bool) -> (FakeConnector, FakeControl) {
    let sent = Sent::default();
    let sent_signal = Arc::new(Notify::new());
    let connect_call = Arc::new(Mutex::new(None));
    let (closed_tx, closed) = oneshot::channel();
    let connector = FakeConnector {
        frames: Arc::new(Mutex::new(VecDeque::from(frames))),
        sent: Arc::clone(&sent),
        sent_signal,
        connect_call: Arc::clone(&connect_call),
        closed: Arc::new(Mutex::new(Some(closed_tx))),
        fail_writes,
    };
    let control = FakeControl {
        sent,
        connect_call,
        closed,
    };
    (connector, control)
}

fn encoded(value: &GuardrailValue) -> Vec<u8> {
    serde_json::to_vec(value).expect("frame JSON")
}

fn empty_chunks() -> Chunks {
    let (sender, receiver) = mpsc::channel(1);
    drop(sender);
    receiver
}

async fn chunks_with(chunks: &[GuardrailValue]) -> Chunks {
    let (sender, receiver) = mpsc::channel(8);
    for chunk in chunks {
        sender.send(Ok(chunk.clone())).await.expect("chunk");
    }
    receiver
}

async fn relay(
    connector: &FakeConnector,
    guardrail: &CatoGuardrail,
    chunks: Chunks,
) -> (Result<(), CatoError>, mpsc::Receiver<GuardrailValue>) {
    let (output, delivered) = mpsc::channel(16);
    let result = time::timeout(
        RELAY_DEADLINE,
        relay_output_stream(connector, guardrail, &HeaderMap::new(), chunks, &output),
    )
    .await
    .expect("the relay finishes");
    drop(output);
    (result, delivered)
}

async fn drain(mut delivered: mpsc::Receiver<GuardrailValue>) -> Vec<GuardrailValue> {
    let mut chunks = Vec::new();
    while let Some(chunk) = delivered.recv().await {
        chunks.push(chunk);
    }
    chunks
}

async fn relay_frames(
    frames: Vec<FakeFrame>,
    fail_writes: bool,
    chunks: Chunks,
) -> (Result<(), CatoError>, Vec<GuardrailValue>) {
    let (connector, _control) = fake_connector(frames, fail_writes);
    let guardrail = ws_guardrail("wss://cato.test", None, Duration::from_secs(5));
    let (result, delivered) = relay(&connector, &guardrail, chunks).await;
    (result, drain(delivered).await)
}

#[tokio::test]
async fn streaming_relay_forwards_the_model_chunks_and_hands_back_the_verified_one() {
    let first = object(&[("choices", list(&[text("first")]))]);
    let second = object(&[("choices", list(&[text("second")]))]);
    let verified = object(&[("verified_chunk", first.clone())]);
    let frames = vec![
        FakeFrame::Message(Ok(Some(encoded(&verified)))),
        FakeFrame::AfterSentTexts(
            4,
            Ok(Some(encoded(&object(&[(
                "done",
                GuardrailValue::Boolean(true),
            )])))),
        ),
    ];
    let (connector, control) = fake_connector(frames, false);
    let guardrail = ws_guardrail("wss://cato.test", None, Duration::from_secs(5));
    let chunks = chunks_with(&[first.clone(), second.clone(), text("raw")]).await;
    let (result, delivered) = relay(&connector, &guardrail, chunks).await;
    result.expect("relayed");
    assert_eq!(drain(delivered).await, vec![first.clone()]);
    let call = {
        let mut call = control.connect_call.lock().expect("connect");
        call.take().expect("the connector was called")
    };
    assert_eq!(call.url, "wss://cato.test/fw/v1/analyze/stream");
    assert_eq!(call.open, CONNECT_TIMEOUT);
    assert_eq!(call.read, Duration::from_secs(5));
    assert_eq!(
        control.sent.lock().expect("sent").clone(),
        vec![
            serde_json::to_string(&first).expect("chunk"),
            serde_json::to_string(&second).expect("chunk"),
            "raw".to_owned(),
            DONE_FRAME.to_owned(),
        ]
    );
}

#[tokio::test]
async fn streaming_relay_raises_a_blocking_message() {
    let frame = object(&[("blocking_message", text("stream blocked"))]);
    let (result, delivered) = relay_frames(
        vec![FakeFrame::Message(Ok(Some(encoded(&frame))))],
        false,
        empty_chunks(),
    )
    .await;
    let error = result.expect_err("blocked");
    assert!(matches!(
        &error,
        CatoError::StreamBlocked { message } if message == "stream blocked"
    ));
    assert_eq!(error.status_code().as_u16(), 500);
    assert!(delivered.is_empty());
}

// LiteLLM returns on done before it looks at a blocking message in the same frame
#[tokio::test]
async fn streaming_relay_lets_done_win_over_a_blocking_message() {
    let frame = object(&[
        ("done", GuardrailValue::Boolean(true)),
        ("blocking_message", text("ignored")),
    ]);
    let (result, delivered) = relay_frames(
        vec![FakeFrame::Message(Ok(Some(encoded(&frame))))],
        false,
        empty_chunks(),
    )
    .await;
    result.expect("done ends the stream");
    assert!(delivered.is_empty());
}

#[tokio::test]
async fn streaming_relay_ends_on_a_frame_it_does_not_recognize() {
    let frame = object(&[("surprise", GuardrailValue::Boolean(true))]);
    let (result, delivered) = relay_frames(
        vec![FakeFrame::Message(Ok(Some(encoded(&frame))))],
        false,
        empty_chunks(),
    )
    .await;
    result.expect("the stream ends");
    assert!(delivered.is_empty());
}

#[tokio::test]
async fn streaming_relay_reports_an_upstream_close() {
    let (result, delivered) =
        relay_frames(vec![FakeFrame::Message(Ok(None))], false, empty_chunks()).await;
    let error = result.expect_err("closed");
    assert!(matches!(error, CatoError::StreamClosed));
    assert_eq!(error.status_code().as_u16(), 500);
    assert!(delivered.is_empty());
}

#[tokio::test]
async fn streaming_relay_raises_on_a_malformed_frame() {
    let frame = FakeFrame::Message(Ok(Some(b"not json".to_vec())));
    let (result, _delivered) = relay_frames(vec![frame], false, empty_chunks()).await;
    let error = result.expect_err("malformed");
    assert!(matches!(error, CatoError::StreamJson { .. }));
    assert_eq!(error.status_code().as_u16(), 500);
}

#[tokio::test]
async fn streaming_relay_reports_a_reader_error() {
    let frame = FakeFrame::Message(Err(CatoError::StreamTimeout));
    let (result, _delivered) = relay_frames(vec![frame], false, empty_chunks()).await;
    let error = result.expect_err("the reader failed");
    assert!(matches!(error, CatoError::StreamTimeout));
}

#[tokio::test]
async fn streaming_relay_reports_a_failed_sender_before_the_next_message() {
    let chunks = chunks_with(&[object(&[("choices", list(&[]))])]).await;
    let (result, _delivered) = relay_frames(Vec::new(), true, chunks).await;
    let error = result.expect_err("the sender failed");
    assert!(matches!(error, CatoError::StreamSenderFailed { .. }));
    assert_eq!(error.status_code().as_u16(), 500);
}

#[tokio::test]
async fn streaming_relay_reports_a_model_stream_that_fails_midway() {
    let chunk = object(&[("choices", list(&[text("one")]))]);
    let (connector, control) = fake_connector(Vec::new(), false);
    let guardrail = ws_guardrail("wss://cato.test", None, Duration::from_secs(5));
    let (sender, chunks) = mpsc::channel(8);
    sender.send(Ok(chunk.clone())).await.expect("chunk");
    let parse_error = serde_json::from_str::<GuardrailValue>("{").expect_err("unterminated");
    let failure = ModelStreamError::Parse {
        source: parse_error,
    };
    sender.send(Err(failure)).await.expect("failure");
    let (result, delivered) = relay(&connector, &guardrail, chunks).await;
    let error = result.expect_err("the model stream failed");
    assert_eq!(error.status_code().as_u16(), 500);
    assert!(matches!(
        &error,
        CatoError::StreamSenderFailed { source }
            if matches!(
                **source,
                CatoError::ModelStream { source: ModelStreamError::Parse { .. } }
            )
    ));
    assert_eq!(
        control.sent.lock().expect("sent").clone(),
        vec![serde_json::to_string(&chunk).expect("chunk")]
    );
    assert!(drain(delivered).await.is_empty());
}

#[tokio::test]
async fn streaming_relay_cancels_the_sender_when_the_client_stops_reading() {
    let verified = object(&[(
        "verified_chunk",
        object(&[("choices", list(&[text("one")]))]),
    )]);
    let (connector, control) = fake_connector(
        vec![FakeFrame::Message(Ok(Some(encoded(&verified))))],
        false,
    );
    let guardrail = ws_guardrail("wss://cato.test", None, Duration::from_secs(5));
    let (output, delivered) = mpsc::channel(1);
    drop(delivered);
    // the model stream stays open, so only a cancelled sender task drops the writer
    let (_model, chunks) = mpsc::channel(1);
    let result =
        relay_output_stream(&connector, &guardrail, &HeaderMap::new(), chunks, &output).await;
    result.expect("the stream ends");
    time::timeout(Duration::from_secs(1), control.closed)
        .await
        .expect("the sender is cancelled")
        .expect_err("the writer was dropped");
}

#[test]
fn cato_stream_limits_follow_the_litellm_transport() {
    assert_eq!(MAX_FRAME_BYTES, 0x0010_0000);
    assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(10));
    assert_eq!(DONE_FRAME, r#"{"done": true}"#);
}
