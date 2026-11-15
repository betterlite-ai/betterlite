#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use google_cloud_speech_v2::model::{
    RecognitionResponseMetadata, StreamingRecognizeRequest, StreamingRecognizeResponse,
    recognition_config::DecodingConfig, streaming_recognize_response::SpeechEventType,
};
use google_cloud_wkt::Duration as ProtoDuration;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::yield_now;
use tokio::time::timeout;

use super::{ChirpBackend, ChirpClock, SpeechClientFactory};
use crate::chirp_backend::error::ChirpBackendError;
use crate::chirp_backend::policy::NORMAL_CLOSURE_CODE;
use crate::chirp_stream::client::{
    ClientFuture, RecognizedStream, RequestSink, ResponseSource, SpeechStreamingClient,
};
use crate::chirp_stream::error::ChirpStreamError;
use crate::chirp_stream::target::{AccessTokenFuture, AccessTokenResolver, SpeechStreamingTarget};

const CONFIGURE: &str = r#"{"kind":"configure","model":"chirp_2","language_codes":["en-US"],"sample_rate_hertz":24000}"#;
const FINISH: &str = r#"{"kind":"finish_turn"}"#;
const DISCARD: &str = r#"{"kind":"discard_turn"}"#;
const RECOGNIZER: &str = "projects/project/locations/us-central1/recognizers/_";

#[derive(Default)]
struct Recorder {
    issued_tokens: Mutex<Vec<String>>,
    sent: Mutex<Vec<SentRequest>>,
    half_closed: Mutex<Vec<usize>>,
    opened: AtomicUsize,
    requests: AtomicUsize,
    half_closes: AtomicUsize,
    closed: AtomicUsize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedConfig {
    model: String,
    language_codes: Vec<String>,
    sample_rate_hertz: i32,
}

#[derive(Clone)]
struct SentRequest {
    stream: usize,
    recognizer: String,
    streaming_config: Option<RecordedConfig>,
    audio: Vec<u8>,
}

fn recorded_config(request: &StreamingRecognizeRequest) -> Option<RecordedConfig> {
    let streaming = request.streaming_config()?;
    let config = streaming.config.as_ref()?;
    let DecodingConfig::ExplicitDecodingConfig(decoding) = config.decoding_config.as_ref()? else {
        return None;
    };
    Some(RecordedConfig {
        model: config.model.clone(),
        language_codes: config.language_codes.clone(),
        sample_rate_hertz: decoding.sample_rate_hertz,
    })
}

impl Recorder {
    fn opened(&self) -> usize {
        self.opened.load(Ordering::SeqCst)
    }

    fn tokens(&self) -> Vec<String> {
        self.issued_tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn sent(&self) -> Vec<SentRequest> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn audio_for(&self, stream: usize) -> Vec<Vec<u8>> {
        self.sent()
            .into_iter()
            .filter(|request| request.stream == stream && !request.audio.is_empty())
            .map(|request| request.audio)
            .collect()
    }

    fn half_closed(&self) -> Vec<usize> {
        self.half_closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct TestClock {
    base: Instant,
    elapsed: Mutex<Duration>,
}

impl TestClock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
        }
    }

    fn advance(&self, by: Duration) {
        let mut elapsed = self.elapsed.lock().unwrap_or_else(PoisonError::into_inner);
        *elapsed = elapsed
            .checked_add(by)
            .expect("the test clock stays in range");
    }
}

impl ChirpClock for TestClock {
    fn now(&self) -> Instant {
        let elapsed = *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner);
        self.base
            .checked_add(elapsed)
            .expect("the test clock stays in range")
    }
}

#[derive(Default)]
struct RecordingTokens {
    issued: AtomicUsize,
}

impl AccessTokenResolver for RecordingTokens {
    fn resolve_access_token(&self) -> AccessTokenFuture<'_> {
        let index = self.issued.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        Box::pin(async move { Ok(SecretString::from(format!("token-{index}"))) })
    }
}

type ScriptedResponse = Option<Result<StreamingRecognizeResponse, ChirpStreamError>>;

struct Script {
    responses: mpsc::Receiver<ScriptedResponse>,
}

fn scripted(capacity: usize) -> (Script, mpsc::Sender<ScriptedResponse>) {
    let (sender, receiver) = mpsc::channel(capacity);
    (
        Script {
            responses: receiver,
        },
        sender,
    )
}

struct Factory {
    recorder: Arc<Recorder>,
    scripts: Mutex<VecDeque<Script>>,
}

impl SpeechClientFactory for Factory {
    fn open<'client>(
        &'client self,
        _target: &'client SpeechStreamingTarget,
        token: SecretString,
    ) -> ClientFuture<'client, Result<Arc<dyn SpeechStreamingClient>, ChirpStreamError>> {
        Box::pin(async move {
            let stream = self.recorder.opened.fetch_add(1, Ordering::SeqCst);
            self.recorder
                .issued_tokens
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(token.expose_secret().to_owned());
            let script = self
                .scripts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .ok_or(ChirpStreamError::ClientClosed)?;
            let client: Arc<dyn SpeechStreamingClient> = Arc::new(TestClient {
                recorder: Arc::clone(&self.recorder),
                stream,
                responses: Mutex::new(Some(script.responses)),
            });
            Ok(client)
        })
    }
}

struct TestClient {
    recorder: Arc<Recorder>,
    stream: usize,
    responses: Mutex<Option<mpsc::Receiver<ScriptedResponse>>>,
}

impl SpeechStreamingClient for TestClient {
    fn streaming_recognize(&self) -> Result<RecognizedStream, ChirpStreamError> {
        let responses = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or(ChirpStreamError::ClientClosed)?;
        Ok(RecognizedStream {
            requests: Box::new(TestSink {
                recorder: Arc::clone(&self.recorder),
                stream: self.stream,
            }),
            responses: Box::new(ChannelResponses { responses }),
        })
    }

    fn close(&self) {
        self.recorder.closed.fetch_add(1, Ordering::SeqCst);
    }
}

struct TestSink {
    recorder: Arc<Recorder>,
    stream: usize,
}

impl RequestSink for TestSink {
    fn send(
        &mut self,
        request: StreamingRecognizeRequest,
    ) -> ClientFuture<'_, Result<(), ChirpStreamError>> {
        Box::pin(async move {
            let audio = request
                .audio()
                .map_or_else(Vec::new, |bytes| bytes.to_vec());
            let streaming_config = recorded_config(&request);
            self.recorder
                .sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(SentRequest {
                    stream: self.stream,
                    recognizer: request.recognizer.clone(),
                    streaming_config,
                    audio,
                });
            self.recorder.requests.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

impl Drop for TestSink {
    fn drop(&mut self) {
        self.recorder
            .half_closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.stream);
        self.recorder.half_closes.fetch_add(1, Ordering::SeqCst);
    }
}

struct ChannelResponses {
    responses: mpsc::Receiver<ScriptedResponse>,
}

impl ResponseSource for ChannelResponses {
    fn next_response(
        &mut self,
    ) -> ClientFuture<'_, Option<Result<StreamingRecognizeResponse, ChirpStreamError>>> {
        Box::pin(async move { self.responses.recv().await.flatten() })
    }
}

struct Harness {
    backend: ChirpBackend,
    clock: Arc<TestClock>,
    recorder: Arc<Recorder>,
    tokens: Arc<RecordingTokens>,
}

fn harness(scripts: Vec<Script>) -> Harness {
    let clock = Arc::new(TestClock::new());
    let recorder = Arc::new(Recorder::default());
    let tokens = Arc::new(RecordingTokens::default());
    let factory = Arc::new(Factory {
        recorder: Arc::clone(&recorder),
        scripts: Mutex::new(VecDeque::from(scripts)),
    });
    let resolver: Arc<dyn AccessTokenResolver> = Arc::<RecordingTokens>::clone(&tokens);
    let target = SpeechStreamingTarget {
        api_endpoint: "speech.googleapis.com".to_owned(),
        recognizer: RECOGNIZER.to_owned(),
        token: resolver,
    };
    let clock_source: Arc<dyn ChirpClock> = Arc::<TestClock>::clone(&clock);
    let backend = ChirpBackend::new(target, factory, clock_source);
    Harness {
        backend,
        clock,
        recorder,
        tokens,
    }
}

impl Harness {
    async fn send_text(&mut self, text: &str) {
        self.backend
            .send(text)
            .await
            .expect("the command is accepted");
    }

    async fn send_audio(&mut self, byte: u8) {
        self.backend
            .send_audio(vec![byte])
            .await
            .expect("the audio is accepted");
    }

    async fn recv(&mut self) -> String {
        self.backend.recv().await.expect("an event is delivered")
    }
}

fn response(event: SpeechEventType, seconds: i64, nanos: i32) -> StreamingRecognizeResponse {
    StreamingRecognizeResponse::new()
        .set_speech_event_type(event)
        .set_metadata(
            RecognitionResponseMetadata::new().set_total_billed_duration(
                ProtoDuration::new(seconds, nanos).expect("a valid duration"),
            ),
        )
}

async fn script_response(
    responses: &mpsc::Sender<ScriptedResponse>,
    response: StreamingRecognizeResponse,
) {
    let queued = responses.send(Some(Ok(response))).await;
    queued.expect("the response is scripted");
}

trait Counted {
    fn at_least(&self, expected: usize) -> bool;
}

impl Counted for &AtomicUsize {
    fn at_least(&self, expected: usize) -> bool {
        self.load(Ordering::SeqCst) >= expected
    }
}

impl<T> Counted for &mut mpsc::Receiver<T> {
    fn at_least(&self, expected: usize) -> bool {
        self.len() >= expected
    }
}

async fn wait_for<C>(counted: C, expected: usize)
where
    C: Counted + Send,
{
    timeout(Duration::from_secs(5), async move {
        while !counted.at_least(expected) {
            yield_now().await;
        }
    })
    .await
    .expect("the backend reaches the expected state");
}

fn assert_opened(harness: &Harness, expected: usize, why: &str) {
    assert_eq!(harness.recorder.opened(), expected, "{why}");
}

#[derive(serde::Deserialize)]
struct BilledEvent {
    billed_seconds: f64,
}

fn billed_seconds(event: &str) -> f64 {
    serde_json::from_str::<BilledEvent>(event)
        .expect("the event is JSON")
        .billed_seconds
}

#[tokio::test]
async fn configure_queues_the_configured_event_without_opening_a_stream() {
    let mut harness = harness(Vec::new());
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    assert_eq!(harness.recorder.opened(), 0);
    assert_eq!(harness.tokens.issued.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn audio_before_configuration_fails() {
    let mut harness = harness(Vec::new());
    let error = harness
        .backend
        .send_audio(vec![1])
        .await
        .expect_err("unconfigured audio is refused");
    assert!(matches!(error, ChirpBackendError::NotConfigured));
    assert_eq!(harness.tokens.issued.load(Ordering::SeqCst), 0);
    assert_eq!(harness.recorder.opened(), 0);
}

#[tokio::test]
async fn audio_sent_to_an_ended_stream_fails_loudly() {
    let (first, first_responses) = scripted(2);
    let mut harness = harness(vec![first]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    drop(first_responses);
    wait_for(&harness.recorder.closed, 1).await;
    let error = harness
        .backend
        .send_audio(vec![2])
        .await
        .expect_err("audio nobody can read is refused");
    assert!(matches!(
        error,
        ChirpBackendError::Stream(ChirpStreamError::QueueClosed)
    ));
    assert_eq!(
        harness.recorder.opened(),
        1,
        "the stream is reused until its rotation threshold"
    );
}

#[tokio::test]
async fn the_first_audio_opens_a_stream_with_the_configured_request() {
    let (script, _responses) = scripted(4);
    let mut harness = harness(vec![script]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(7).await;
    wait_for(&harness.recorder.requests, 2).await;
    let sent = harness.recorder.sent();
    assert_eq!(sent[0].recognizer, RECOGNIZER);
    assert_eq!(
        sent[0].streaming_config,
        Some(RecordedConfig {
            model: "chirp_2".to_owned(),
            language_codes: vec!["en-US".to_owned()],
            sample_rate_hertz: 24000,
        }),
        "the first request carries the configure command's streaming settings"
    );
    assert!(sent[0].audio.is_empty());
    assert_eq!(sent[1].streaming_config, None);
    assert_eq!(sent[1].audio, vec![7]);
    assert_eq!(harness.recorder.tokens(), vec!["token-1".to_owned()]);
}

#[tokio::test]
async fn rotation_waits_for_silence() {
    let (first, first_responses) = scripted(4);
    let (second, _second_responses) = scripted(4);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    script_response(
        &first_responses,
        response(SpeechEventType::SpeechActivityBegin, 0, 500_000_000),
    )
    .await;
    assert!(harness.recv().await.contains("\"begin\""));
    harness.clock.advance(Duration::from_secs(240));
    harness.send_audio(2).await;
    assert_opened(&harness, 1, "speech blocks the soft rotation");
    harness.clock.advance(Duration::from_secs(1));
    harness.send_audio(3).await;
    assert_opened(&harness, 1, "an active stream stays below the deadline");
    harness.clock.advance(Duration::from_secs(39));
    harness.send_audio(4).await;
    assert_opened(&harness, 2, "the hard deadline rotates a speaking stream");
    wait_for(&harness.recorder.half_closes, 1).await;
    assert_eq!(harness.recorder.half_closed(), vec![0]);
    drop(first_responses);
    wait_for(&harness.recorder.requests, 6).await;
    assert_eq!(
        harness.recorder.audio_for(0),
        vec![vec![1], vec![2], vec![3]]
    );
    assert_eq!(harness.recorder.audio_for(1), vec![vec![4]]);
}

#[tokio::test]
async fn hard_deadline_rotation() {
    let (first, first_responses) = scripted(4);
    let (second, _second_responses) = scripted(4);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    script_response(
        &first_responses,
        response(SpeechEventType::SpeechActivityBegin, 0, 250_000_000),
    )
    .await;
    assert!(harness.recv().await.contains("\"begin\""));
    harness.clock.advance(Duration::from_secs(279));
    harness.send_audio(2).await;
    assert_eq!(harness.recorder.opened(), 1);
    harness.clock.advance(Duration::from_secs(1));
    harness.send_audio(3).await;
    assert_eq!(harness.recorder.opened(), 2);
}

#[tokio::test]
async fn an_inactive_stream_rotates_at_the_soft_threshold() {
    let (first, _first_responses) = scripted(2);
    let (second, _second_responses) = scripted(2);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    harness.clock.advance(Duration::from_millis(239_999));
    harness.send_audio(2).await;
    assert_eq!(harness.recorder.opened(), 1);
    harness.clock.advance(Duration::from_millis(1));
    harness.send_audio(3).await;
    assert_eq!(harness.recorder.opened(), 2);
}

#[tokio::test]
async fn finish_fifo() {
    let (first, responses) = scripted(4);
    let mut harness = harness(vec![first]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    script_response(
        &responses,
        response(SpeechEventType::Unspecified, 0, 250_000_000),
    )
    .await;
    assert!(harness.recv().await.contains("\"response\""));
    harness.send_text(FINISH).await;
    wait_for(&harness.recorder.half_closes, 1).await;
    assert_eq!(
        harness.recorder.half_closed(),
        vec![0],
        "finishing half-closes the turn's last stream while its relay still runs"
    );
    script_response(
        &responses,
        response(SpeechEventType::Unspecified, 0, 250_000_000),
    )
    .await;
    assert!(
        harness.recv().await.contains("\"response\""),
        "a stream result queued before turn_finished is delivered first"
    );
    drop(responses);
    assert_eq!(harness.recv().await, r#"{"kind":"turn_finished"}"#);
}

#[tokio::test]
async fn finish_half_closes_only_the_last_stream() {
    let (first, first_responses) = scripted(2);
    let (second, _second_responses) = scripted(2);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    harness.clock.advance(Duration::from_secs(240));
    harness.send_audio(2).await;
    wait_for(&harness.recorder.half_closes, 1).await;
    assert_eq!(
        harness.recorder.half_closed(),
        vec![0],
        "the rotation half-closes the replaced stream"
    );
    harness.send_text(FINISH).await;
    drop(first_responses);
    wait_for(&harness.recorder.half_closes, 2).await;
    assert_eq!(
        harness.recorder.half_closed(),
        vec![0, 1],
        "finish half-closes the last stream after the rotation half-closed the replaced one"
    );
}

#[tokio::test]
async fn discard_suppresses_old_events() {
    let (first, first_responses) = scripted(2);
    let (second, second_responses) = scripted(2);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    script_response(
        &first_responses,
        response(SpeechEventType::Unspecified, 0, 500_000_000),
    )
    .await;
    wait_for(&mut harness.backend.outbox, 1).await;
    harness.clock.advance(Duration::from_secs(240));
    harness.send_audio(2).await;
    drop(first_responses);
    wait_for(&harness.recorder.closed, 1).await;
    script_response(
        &second_responses,
        response(SpeechEventType::Unspecified, 0, 250_000_000),
    )
    .await;
    wait_for(&mut harness.backend.outbox, 2).await;
    harness.send_text(DISCARD).await;
    let marker = timeout(Duration::from_secs(5), harness.backend.recv())
        .await
        .expect("the discard marker reaches the caller before the second stream's channel closes")
        .expect("the marker is delivered");
    assert_eq!(
        marker, r#"{"kind":"turn_discarded","billed_seconds":0.75}"#,
        "the discarded turn's results never reach the caller and both streams bill once"
    );
    wait_for(&harness.recorder.closed, 2).await;
    drop(second_responses);
}

#[tokio::test]
async fn a_discarded_turn_leaves_the_next_turn_deliverable() {
    let (first, first_responses) = scripted(2);
    let (second, second_responses) = scripted(2);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    drop(first_responses);
    wait_for(&harness.recorder.closed, 1).await;
    harness.send_text(DISCARD).await;
    assert_eq!(
        harness.recv().await,
        r#"{"kind":"turn_discarded","billed_seconds":0.0}"#
    );
    harness.send_audio(2).await;
    script_response(
        &second_responses,
        response(SpeechEventType::Unspecified, 0, 250_000_000),
    )
    .await;
    assert!(
        harness.recv().await.contains("\"response\""),
        "the turn after a discard delivers its results"
    );
}

#[tokio::test]
async fn billing_accumulates_across_turns() {
    let (first, first_responses) = scripted(2);
    let (second, second_responses) = scripted(2);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    script_response(
        &first_responses,
        response(SpeechEventType::Unspecified, 2, 0),
    )
    .await;
    assert_eq!(
        billed_seconds(&harness.recv().await).to_bits(),
        2.0_f64.to_bits()
    );
    harness.send_text(FINISH).await;
    drop(first_responses);
    assert_eq!(harness.recv().await, r#"{"kind":"turn_finished"}"#);
    harness.send_audio(2).await;
    script_response(
        &second_responses,
        response(SpeechEventType::Unspecified, 1, 0),
    )
    .await;
    assert_eq!(
        billed_seconds(&harness.recv().await).to_bits(),
        3.0_f64.to_bits(),
        "the second turn's result carries both turns' billed seconds"
    );
    drop(second_responses);
}

#[tokio::test]
async fn fresh_token_each_stream() {
    let (first, _first_responses) = scripted(2);
    let (second, _second_responses) = scripted(2);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    assert_eq!(harness.tokens.issued.load(Ordering::SeqCst), 1);
    assert_eq!(harness.recorder.tokens(), vec!["token-1".to_owned()]);
    harness.clock.advance(Duration::from_secs(240));
    harness.send_audio(2).await;
    assert_eq!(harness.tokens.issued.load(Ordering::SeqCst), 2);
    assert_eq!(
        harness.recorder.tokens(),
        vec!["token-1".to_owned(), "token-2".to_owned()]
    );
}

#[tokio::test]
async fn rotation_and_finish_tolerate_a_stream_that_already_ended() {
    let (first, first_responses) = scripted(2);
    let (second, second_responses) = scripted(2);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    drop(first_responses);
    wait_for(&harness.recorder.closed, 1).await;
    harness.clock.advance(Duration::from_secs(240));
    harness.send_audio(2).await;
    assert_eq!(
        harness.recorder.opened(),
        2,
        "an expired stream whose relay already ended still rotates"
    );
    assert_eq!(harness.tokens.issued.load(Ordering::SeqCst), 2);
    drop(second_responses);
    wait_for(&harness.recorder.closed, 2).await;
    harness.send_text(FINISH).await;
    assert_eq!(
        harness.recv().await,
        r#"{"kind":"turn_finished"}"#,
        "finishing a turn whose last stream already ended still reports the turn"
    );
}

#[tokio::test]
async fn a_stream_failure_closes_with_the_stream_failure_code() {
    let (first, responses) = scripted(2);
    let mut harness = harness(vec![first]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    responses
        .send(Some(Err(ChirpStreamError::ClientClosed)))
        .await
        .expect("the failure is scripted");
    drop(responses);
    let error = harness
        .backend
        .recv()
        .await
        .expect_err("the failure closes the session");
    match error {
        ChirpBackendError::Closed { code, reason } => {
            assert_eq!(code, 1011);
            assert!(
                reason.ends_with("the Speech-to-Text client is closed"),
                "the reason names the stream failure: {reason}"
            );
        }
        other @ (ChirpBackendError::Command(_)
        | ChirpBackendError::Encode(_)
        | ChirpBackendError::NotConfigured
        | ChirpBackendError::Token(_)
        | ChirpBackendError::QueueClosed
        | ChirpBackendError::Stream(_)) => {
            panic!("expected a stream failure close, got {other:?}")
        }
    }
}

#[tokio::test]
async fn close_queues_the_closed_sentinel_after_buffered_events() {
    let mut harness = harness(Vec::new());
    harness.send_text(FINISH).await;
    wait_for(&mut harness.backend.outbox, 1).await;
    harness.backend.close().await;
    harness.backend.close().await;
    assert_eq!(
        harness.backend.outbox.len(),
        2,
        "close queues the closed sentinel behind the buffered event"
    );
    assert_eq!(harness.recv().await, r#"{"kind":"turn_finished"}"#);
    let error = harness
        .backend
        .recv()
        .await
        .expect_err("a closed backend stops delivering");
    assert!(matches!(
        error,
        ChirpBackendError::Closed {
            code: NORMAL_CLOSURE_CODE,
            ..
        }
    ));
}

async fn fill_outbox(harness: &mut Harness) {
    for _ in 0..256_usize {
        harness.send_text(FINISH).await;
    }
}

async fn drain_outbox(harness: &mut Harness) {
    for _ in 0..256_usize {
        assert_eq!(harness.recv().await, r#"{"kind":"turn_finished"}"#);
    }
}

#[tokio::test]
async fn full_outbox_close() {
    let mut harness = harness(Vec::new());
    fill_outbox(&mut harness).await;
    wait_for(&mut harness.backend.outbox, 256).await;
    harness.backend.close().await;
    assert_eq!(
        harness.backend.outbox.len(),
        256,
        "a full outbox leaves no room for the closed sentinel"
    );
    drain_outbox(&mut harness).await;
    let error = harness
        .backend
        .recv()
        .await
        .expect_err("a full outbox still ends with the normal closure");
    assert!(matches!(
        error,
        ChirpBackendError::Closed {
            code: NORMAL_CLOSURE_CODE,
            ..
        }
    ));
}

#[tokio::test]
async fn a_closed_backend_refuses_further_messages() {
    let mut harness = harness(Vec::new());
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.backend.close().await;
    harness.backend.close().await;
    let command = harness
        .backend
        .send(CONFIGURE)
        .await
        .expect_err("a closed backend refuses commands");
    assert!(matches!(
        command,
        ChirpBackendError::Closed {
            code: NORMAL_CLOSURE_CODE,
            ..
        }
    ));
    let audio = harness
        .backend
        .send_audio(vec![1])
        .await
        .expect_err("a closed backend refuses audio");
    assert!(matches!(
        audio,
        ChirpBackendError::Closed {
            code: NORMAL_CLOSURE_CODE,
            ..
        }
    ));
}

#[tokio::test]
async fn close_closes_a_stream_still_waiting_in_the_link_queue() {
    let (first, first_responses) = scripted(2);
    let (second, _second_responses) = scripted(2);
    let mut harness = harness(vec![first, second]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    harness.clock.advance(Duration::from_secs(240));
    harness.send_audio(2).await;
    assert_eq!(
        harness.recorder.opened(),
        2,
        "the second stream is opened while the pump still relays the first"
    );
    harness.backend.close().await;
    wait_for(&harness.recorder.closed, 2).await;
    assert_eq!(
        harness.recorder.closed.load(Ordering::SeqCst),
        2,
        "close reaches the relayed stream and the one still in the link queue"
    );
    drop(first_responses);
}

#[tokio::test]
async fn no_task_leak() {
    let baseline = Handle::current().metrics().num_alive_tasks();
    let (first, responses) = scripted(2);
    let mut harness = harness(vec![first]);
    harness.send_text(CONFIGURE).await;
    assert_eq!(harness.recv().await, r#"{"kind":"configured"}"#);
    harness.send_audio(1).await;
    assert!(
        Handle::current().metrics().num_alive_tasks() > baseline,
        "the link pump and the stream relay run as owned tasks"
    );
    drop(responses);
    harness.backend.close().await;
    assert_eq!(
        Handle::current().metrics().num_alive_tasks(),
        baseline,
        "close joins every task the backend started"
    );
}
