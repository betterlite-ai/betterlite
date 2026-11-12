#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use google_cloud_speech_v2::model::{
    RecognitionResponseMetadata, StreamingRecognizeRequest, StreamingRecognizeResponse,
    streaming_recognize_response::SpeechEventType,
};
use google_cloud_wkt::Duration as ProtoDuration;
use jiff::SignedDuration;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::RecognizeStream;
use crate::chirp_stream::client::{
    ClientFuture, RecognizedStream, RequestSink, ResponseSource, SpeechStreamingClient,
};
use crate::chirp_stream::error::ChirpStreamError;
use crate::chirp_stream::limits::OUTBOX_SIZE;
use crate::chirp_stream::outbox::OutboxItem;
use crate::chirp_stream::records::ChirpStreamEvent;

#[derive(Debug, PartialEq, Eq)]
enum Sent {
    Request { audio: Vec<u8> },
    HalfClosed,
}

#[derive(Debug, Default)]
struct Recorder {
    sent: Mutex<Vec<Sent>>,
    started: AtomicUsize,
    closed: AtomicUsize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    Send,
    Open,
}

struct TestClient {
    recorder: Arc<Recorder>,
    half_closed: Arc<Notify>,
    responses: Mutex<Option<Box<dyn ResponseSource>>>,
    fault: Fault,
}

impl SpeechStreamingClient for TestClient {
    fn streaming_recognize(&self) -> Result<RecognizedStream, ChirpStreamError> {
        self.recorder.started.fetch_add(1, Ordering::SeqCst);
        if self.fault == Fault::Open {
            return Err(ChirpStreamError::ClientClosed);
        }
        let responses = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or(ChirpStreamError::ClientClosed)?;
        Ok(RecognizedStream {
            requests: Box::new(TestSink {
                recorder: Arc::clone(&self.recorder),
                half_closed: Arc::clone(&self.half_closed),
                fail: self.fault == Fault::Send,
            }),
            responses,
        })
    }

    fn close(&self) {
        self.recorder.closed.fetch_add(1, Ordering::SeqCst);
    }
}

struct TestSink {
    recorder: Arc<Recorder>,
    half_closed: Arc<Notify>,
    fail: bool,
}

impl RequestSink for TestSink {
    fn send(
        &mut self,
        request: StreamingRecognizeRequest,
    ) -> ClientFuture<'_, Result<(), ChirpStreamError>> {
        Box::pin(async move {
            if self.fail {
                return Err(ChirpStreamError::ClientClosed);
            }
            let audio = request
                .audio()
                .map_or_else(Vec::new, |bytes| bytes.to_vec());
            self.recorder
                .sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Sent::Request { audio });
            Ok(())
        })
    }
}

impl Drop for TestSink {
    fn drop(&mut self) {
        self.recorder
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent::HalfClosed);
        self.half_closed.notify_one();
    }
}

struct ChannelResponses {
    responses: mpsc::Receiver<Option<Result<StreamingRecognizeResponse, ChirpStreamError>>>,
}

impl ResponseSource for ChannelResponses {
    fn next_response(
        &mut self,
    ) -> ClientFuture<'_, Option<Result<StreamingRecognizeResponse, ChirpStreamError>>> {
        Box::pin(async move { self.responses.recv().await.flatten() })
    }
}

struct FinishAfterHalfClose {
    half_closed: Arc<Notify>,
    emitted: bool,
}

impl ResponseSource for FinishAfterHalfClose {
    fn next_response(
        &mut self,
    ) -> ClientFuture<'_, Option<Result<StreamingRecognizeResponse, ChirpStreamError>>> {
        Box::pin(async move {
            if self.emitted {
                return None;
            }
            self.half_closed.notified().await;
            self.emitted = true;
            None
        })
    }
}

fn first_request() -> StreamingRecognizeRequest {
    StreamingRecognizeRequest::new().set_recognizer("projects/project/locations/l/recognizers/_")
}

fn client(
    recorder: &Arc<Recorder>,
    half_closed: &Arc<Notify>,
    responses: Box<dyn ResponseSource>,
    fault: Fault,
) -> Arc<dyn SpeechStreamingClient> {
    Arc::new(TestClient {
        recorder: Arc::clone(recorder),
        half_closed: Arc::clone(half_closed),
        responses: Mutex::new(Some(responses)),
        fault,
    })
}

fn stream(client: Arc<dyn SpeechStreamingClient>) -> Arc<RecognizeStream> {
    Arc::new(RecognizeStream::new(client, first_request(), Instant::now(), 7).expect("a stream"))
}

type Scripted = mpsc::Sender<Option<Result<StreamingRecognizeResponse, ChirpStreamError>>>;

fn scripting_stream(
    recorder: &Arc<Recorder>,
    half_closed: &Arc<Notify>,
    capacity: usize,
    fault: Fault,
) -> (Arc<RecognizeStream>, Scripted) {
    let (responses_tx, responses_rx) = mpsc::channel(capacity);
    let responses = Box::new(ChannelResponses {
        responses: responses_rx,
    });
    (
        stream(client(recorder, half_closed, responses, fault)),
        responses_tx,
    )
}

async fn join_relay(mut tasks: JoinSet<SignedDuration>) -> SignedDuration {
    timeout(Duration::from_secs(5), tasks.join_next())
        .await
        .expect("the relay finishes")
        .expect("the relay task finishes")
        .expect("the relay does not panic")
}

fn start_relay(
    stream: &Arc<RecognizeStream>,
    billed_before: f64,
) -> (mpsc::Receiver<OutboxItem>, JoinSet<SignedDuration>) {
    let (outbox_tx, outbox_rx) = mpsc::channel(OUTBOX_SIZE);
    let relayed = Arc::clone(stream);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { relayed.relay(outbox_tx, billed_before).await });
    (outbox_rx, tasks)
}

async fn expect_turn(outbox_rx: &mut mpsc::Receiver<OutboxItem>) {
    assert!(matches!(
        outbox_rx.recv().await,
        Some(OutboxItem::TurnResult { .. })
    ));
}

async fn relayed(
    stream: &Arc<RecognizeStream>,
    billed_before: f64,
) -> (SignedDuration, mpsc::Receiver<OutboxItem>) {
    let (outbox_rx, tasks) = start_relay(stream, billed_before);
    let billed = join_relay(tasks).await;
    (billed, outbox_rx)
}

async fn script_response(
    responses_tx: &Scripted,
    event: SpeechEventType,
    seconds: i64,
    nanos: i32,
) {
    responses_tx
        .send(Some(Ok(billing_response(event, seconds, nanos))))
        .await
        .expect("the response is scripted");
}

async fn fill_queue(stream: &Arc<RecognizeStream>) {
    for _ in 0..63_usize {
        stream.send_audio(vec![0]).await.expect("queue space");
    }
}

#[tokio::test]
async fn queue_backpressure_holds_the_sixty_fourth_audio_chunk() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 1, Fault::None);
    fill_queue(&stream).await;
    let mut blocked = Box::pin(stream.send_audio(vec![0]));
    assert!(
        timeout(Duration::ZERO, &mut blocked).await.is_err(),
        "the queue is full and the send waits"
    );
    let (outbox_tx, mut outbox_rx) = mpsc::channel(OUTBOX_SIZE);
    let mut tasks = JoinSet::new();
    let relayed = Arc::clone(&stream);
    tasks.spawn(async move { relayed.relay(outbox_tx, 0.0).await });
    timeout(Duration::from_secs(5), &mut blocked)
        .await
        .expect("the relay drains a queue slot")
        .expect("the audio is queued");
    drop(responses_tx);
    let billed = tasks
        .join_next()
        .await
        .expect("the relay task finishes")
        .expect("the relay does not panic");
    assert_eq!(billed, SignedDuration::ZERO);
    assert_eq!(recorder.started.load(Ordering::SeqCst), 1);
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
    assert_eq!(
        outbox_rx.try_recv().expect_err("no events were forwarded"),
        mpsc::error::TryRecvError::Disconnected
    );
}

#[tokio::test]
async fn half_close_waits_behind_the_queued_audio() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let stream = stream(client(
        &recorder,
        &half_closed,
        Box::new(FinishAfterHalfClose {
            half_closed: Arc::clone(&half_closed),
            emitted: false,
        }),
        Fault::None,
    ));
    stream.send_audio(vec![1, 2]).await.expect("queue space");
    stream.send_audio(vec![3]).await.expect("queue space");
    stream.half_close().await.expect("the terminator is queued");
    let (outbox_tx, _outbox_rx) = mpsc::channel(OUTBOX_SIZE);
    let billed = timeout(Duration::from_secs(5), stream.relay(outbox_tx, 0.0))
        .await
        .expect("the relay finishes");
    assert_eq!(billed, SignedDuration::ZERO);
    assert_eq!(
        *recorder.sent.lock().unwrap_or_else(PoisonError::into_inner),
        [
            Sent::Request { audio: Vec::new() },
            Sent::Request { audio: vec![1, 2] },
            Sent::Request { audio: vec![3] },
            Sent::HalfClosed,
        ]
    );
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelling_a_running_relay_stops_it() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 1, Fault::None);
    let (mut outbox_rx, tasks) = start_relay(&stream, 0.0);
    script_response(&responses_tx, SpeechEventType::Unspecified, 0, 250_000_000).await;
    expect_turn(&mut outbox_rx).await;
    stream.halt.cancel();
    let billed = join_relay(tasks).await;
    assert_eq!(billed, SignedDuration::new(0, 250_000_000));
    assert_eq!(
        outbox_rx
            .try_recv()
            .expect_err("no result follows the cancel"),
        mpsc::error::TryRecvError::Disconnected
    );
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
    assert!(matches!(
        stream.send_audio(vec![0]).await,
        Err(ChirpStreamError::QueueClosed)
    ));
    drop(responses_tx);
}

#[tokio::test]
async fn the_client_closes_only_after_the_response_stream_ends() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 1, Fault::None);
    stream.half_close().await.expect("the terminator is queued");
    let (mut outbox_rx, tasks) = start_relay(&stream, 0.0);
    timeout(Duration::from_secs(5), half_closed.notified())
        .await
        .expect("the request side half-closes");
    assert_eq!(
        recorder.closed.load(Ordering::SeqCst),
        0,
        "half-close must not close a stream that still has responses"
    );
    script_response(&responses_tx, SpeechEventType::Unspecified, 0, 250_000_000).await;
    expect_turn(&mut outbox_rx).await;
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 0);
    drop(responses_tx);
    let billed = join_relay(tasks).await;
    assert_eq!(billed, SignedDuration::new(0, 250_000_000));
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_cancelled_stream_closes_without_starting() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (_responses_tx, responses_rx) = mpsc::channel(1);
    let stream = stream(client(
        &recorder,
        &half_closed,
        Box::new(ChannelResponses {
            responses: responses_rx,
        }),
        Fault::None,
    ));
    stream.halt.cancel();
    let (outbox_tx, _outbox_rx) = mpsc::channel(OUTBOX_SIZE);
    let billed = stream.relay(outbox_tx, 0.0).await;
    assert_eq!(billed, SignedDuration::ZERO);
    assert_eq!(recorder.started.load(Ordering::SeqCst), 0);
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_response_failure_enqueues_the_reason_and_closes() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 1, Fault::None);
    responses_tx
        .send(Some(Err(ChirpStreamError::ClientClosed)))
        .await
        .expect("the response is scripted");
    drop(responses_tx);
    let (billed, mut outbox_rx) = relayed(&stream, 2.5).await;
    assert_eq!(billed, SignedDuration::ZERO);
    let failure = outbox_rx.recv().await.expect("a failure item");
    match failure {
        OutboxItem::StreamFailure { reason } => assert_eq!(
            reason,
            "Google Speech-to-Text streaming failed: the Speech-to-Text client is closed"
        ),
        OutboxItem::Text(_)
        | OutboxItem::TurnResult { .. }
        | OutboxItem::TurnDiscardedEvent { .. }
        | OutboxItem::Closed => panic!("expected a stream failure"),
    }
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_send_failure_enqueues_the_reason_and_closes() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let stream = stream(client(
        &recorder,
        &half_closed,
        Box::new(FinishAfterHalfClose {
            half_closed: Arc::clone(&half_closed),
            emitted: false,
        }),
        Fault::Send,
    ));
    let (billed, mut outbox_rx) = relayed(&stream, 0.0).await;
    assert_eq!(billed, SignedDuration::ZERO);
    assert!(matches!(
        outbox_rx.recv().await,
        Some(OutboxItem::StreamFailure { .. })
    ));
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_stream_that_cannot_open_enqueues_the_reason_and_closes() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (_responses_tx, responses_rx) = mpsc::channel(1);
    let stream = stream(client(
        &recorder,
        &half_closed,
        Box::new(ChannelResponses {
            responses: responses_rx,
        }),
        Fault::Open,
    ));
    let (billed, mut outbox_rx) = relayed(&stream, 0.0).await;
    assert_eq!(billed, SignedDuration::ZERO);
    match outbox_rx.recv().await.expect("a failure item") {
        OutboxItem::StreamFailure { reason } => assert_eq!(
            reason,
            "Google Speech-to-Text streaming failed: the Speech-to-Text client is closed"
        ),
        OutboxItem::Text(_)
        | OutboxItem::TurnResult { .. }
        | OutboxItem::TurnDiscardedEvent { .. }
        | OutboxItem::Closed => panic!("expected a stream failure"),
    }
    assert_eq!(recorder.started.load(Ordering::SeqCst), 1);
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_begin_makes_the_stream_active() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 1, Fault::None);
    assert!(
        !stream.speech_active.load(Ordering::SeqCst),
        "a new stream starts inactive"
    );
    stream.half_close().await.expect("the terminator is queued");
    let (mut outbox_rx, tasks) = start_relay(&stream, 0.5);
    script_response(
        &responses_tx,
        SpeechEventType::SpeechActivityBegin,
        1,
        250_000_000,
    )
    .await;
    expect_turn(&mut outbox_rx).await;
    assert!(
        stream.speech_active.load(Ordering::SeqCst),
        "a begin makes the stream active"
    );
    drop(responses_tx);
    let billed = join_relay(tasks).await;
    assert_eq!(billed, SignedDuration::new(1, 250_000_000));
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_end_makes_the_stream_inactive() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 1, Fault::None);
    stream.half_close().await.expect("the terminator is queued");
    let (mut outbox_rx, tasks) = start_relay(&stream, 0.0);
    script_response(
        &responses_tx,
        SpeechEventType::SpeechActivityBegin,
        1,
        250_000_000,
    )
    .await;
    expect_turn(&mut outbox_rx).await;
    assert!(stream.speech_active.load(Ordering::SeqCst));
    script_response(
        &responses_tx,
        SpeechEventType::SpeechActivityEnd,
        0,
        500_000_000,
    )
    .await;
    expect_turn(&mut outbox_rx).await;
    assert!(
        !stream.speech_active.load(Ordering::SeqCst),
        "an end makes the stream inactive"
    );
    drop(responses_tx);
    let billed = join_relay(tasks).await;
    assert_eq!(billed, SignedDuration::new(1, 250_000_000));
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_response_without_an_activity_event_keeps_the_stream_active() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 1, Fault::None);
    stream.half_close().await.expect("the terminator is queued");
    let (mut outbox_rx, tasks) = start_relay(&stream, 0.0);
    script_response(
        &responses_tx,
        SpeechEventType::SpeechActivityBegin,
        1,
        250_000_000,
    )
    .await;
    expect_turn(&mut outbox_rx).await;
    assert!(stream.speech_active.load(Ordering::SeqCst));
    script_response(&responses_tx, SpeechEventType::Unspecified, 0, 500_000_000).await;
    expect_turn(&mut outbox_rx).await;
    assert!(
        stream.speech_active.load(Ordering::SeqCst),
        "a response without an activity event leaves the state alone"
    );
    drop(responses_tx);
    let billed = join_relay(tasks).await;
    assert_eq!(billed, SignedDuration::new(1, 250_000_000));
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_cancel_aborts_a_blocked_failure_send() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 1, Fault::None);
    responses_tx
        .send(Some(Err(ChirpStreamError::ClientClosed)))
        .await
        .expect("the error is buffered");
    let (outbox_tx, mut outbox_rx) = mpsc::channel(1);
    outbox_tx
        .send(OutboxItem::Closed)
        .await
        .expect("the outbox is full");
    let relayed_stream = Arc::clone(&stream);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { relayed_stream.relay(outbox_tx, 0.0).await });
    let _second = timeout(Duration::from_secs(5), responses_tx.send(None))
        .await
        .expect("the relay consumed the error");
    stream.halt.cancel();
    let billed = join_relay(tasks).await;
    assert_eq!(billed, SignedDuration::ZERO);
    assert!(matches!(outbox_rx.try_recv(), Ok(OutboxItem::Closed)));
    assert!(matches!(
        outbox_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
    assert_eq!(recorder.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn results_carry_the_cumulative_billed_maximum() {
    let recorder = Arc::new(Recorder::default());
    let half_closed = Arc::new(Notify::new());
    let (stream, responses_tx) = scripting_stream(&recorder, &half_closed, 4, Fault::None);
    responses_tx
        .send(Some(Ok(billing_response(
            SpeechEventType::SpeechActivityBegin,
            1,
            250_000_000,
        ))))
        .await
        .expect("the first response is scripted");
    responses_tx
        .send(Some(Ok(billing_response(
            SpeechEventType::Unspecified,
            0,
            250_000_000,
        ))))
        .await
        .expect("the second response is scripted");
    drop(responses_tx);
    let (billed, mut outbox_rx) = relayed(&stream, 0.5).await;
    assert_eq!(billed, SignedDuration::new(1, 250_000_000));
    let first = outbox_rx.recv().await.expect("the first result");
    let second = outbox_rx.recv().await.expect("the second result");
    let (first_turn, first_event) = turn_result(first);
    let (second_turn, second_event) = turn_result(second);
    assert_eq!(first_turn, 7);
    assert_eq!(second_turn, 7);
    assert_eq!(first_event.billed_seconds.to_bits(), 1.75_f64.to_bits());
    assert_eq!(second_event.billed_seconds.to_bits(), 1.75_f64.to_bits());
}

fn turn_result(item: OutboxItem) -> (usize, ChirpStreamEvent) {
    match item {
        OutboxItem::TurnResult { turn, event } => (turn, event),
        OutboxItem::Text(_)
        | OutboxItem::TurnDiscardedEvent { .. }
        | OutboxItem::StreamFailure { .. }
        | OutboxItem::Closed => panic!("expected a turn result"),
    }
}

fn billing_response(
    event: SpeechEventType,
    seconds: i64,
    nanos: i32,
) -> StreamingRecognizeResponse {
    StreamingRecognizeResponse::new()
        .set_speech_event_type(event)
        .set_metadata(
            RecognitionResponseMetadata::new().set_total_billed_duration(
                ProtoDuration::new(seconds, nanos).expect("a valid duration"),
            ),
        )
}
