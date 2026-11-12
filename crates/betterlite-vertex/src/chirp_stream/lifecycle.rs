use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use google_cloud_speech_v2::model::{StreamingRecognizeRequest, StreamingRecognizeResponse};
use jiff::SignedDuration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::client::{RecognizedStream, RequestSink, ResponseSource, SpeechStreamingClient};
use super::error::ChirpStreamError;
use super::limits::REQUEST_QUEUE_SIZE;
use super::outbox::OutboxItem;
use super::records::{SpeechActivity, billed_duration, response_event, speech_activity};

mod tests;

pub struct Connection {
    client: Arc<dyn SpeechStreamingClient>,
    closed: AtomicBool,
}

impl Connection {
    pub fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.client.close();
        }
    }
}

#[expect(
    clippy::partial_pub_fields,
    reason = "the consumer rotates on the open time, turn and activity and stops the stream, while the request queue and billing stay internal"
)]
pub struct RecognizeStream {
    pub connection: Connection,
    pub opened_at: Instant,
    pub turn: usize,
    pub speech_active: Arc<AtomicBool>,
    pub halt: CancellationToken,
    requests: mpsc::Sender<Option<StreamingRecognizeRequest>>,
    receiver: Mutex<Option<mpsc::Receiver<Option<StreamingRecognizeRequest>>>>,
    billed: Arc<Mutex<SignedDuration>>,
}

impl RecognizeStream {
    pub fn new(
        client: Arc<dyn SpeechStreamingClient>,
        first_request: StreamingRecognizeRequest,
        opened_at: Instant,
        turn: usize,
    ) -> Result<Self, ChirpStreamError> {
        let (requests, receiver) = mpsc::channel(REQUEST_QUEUE_SIZE);
        requests
            .try_send(Some(first_request))
            .map_err(|_source| ChirpStreamError::QueueClosed)?;
        Ok(Self {
            connection: Connection {
                client,
                closed: AtomicBool::new(false),
            },
            opened_at,
            turn,
            speech_active: Arc::new(AtomicBool::new(false)),
            halt: CancellationToken::new(),
            requests,
            receiver: Mutex::new(Some(receiver)),
            billed: Arc::new(Mutex::new(SignedDuration::ZERO)),
        })
    }

    pub async fn send_audio(&self, audio: Vec<u8>) -> Result<(), ChirpStreamError> {
        self.requests
            .send(Some(StreamingRecognizeRequest::new().set_audio(audio)))
            .await
            .map_err(|_source| ChirpStreamError::QueueClosed)
    }

    pub async fn half_close(&self) -> Result<(), ChirpStreamError> {
        self.requests
            .send(None)
            .await
            .map_err(|_source| ChirpStreamError::QueueClosed)
    }

    pub async fn relay(
        &self,
        outbox: mpsc::Sender<OutboxItem>,
        billed_before: f64,
    ) -> SignedDuration {
        if self.halt.is_cancelled() {
            self.connection.close();
            return SignedDuration::ZERO;
        }
        self.forward(outbox, billed_before).await;
        self.connection.close();
        *self.billed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn forward(&self, outbox: mpsc::Sender<OutboxItem>, billed_before: f64) {
        let Some(queue) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let failure = Arc::new(Mutex::new(None));
        let Some(stream) = self.open_stream(&failure) else {
            report_failure(&outbox, &failure).await;
            return;
        };
        let mut tasks = JoinSet::new();
        tasks.spawn(
            Drain {
                queue,
                sink: stream.requests,
                failure: Arc::clone(&failure),
                halt: self.halt.clone(),
            }
            .run(),
        );
        self.forward_responses(stream.responses, &outbox, billed_before, &failure)
            .await;
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        report_failure(&outbox, &failure).await;
    }

    fn open_stream(&self, failure: &Mutex<Option<String>>) -> Option<RecognizedStream> {
        match self.connection.client.streaming_recognize() {
            Ok(stream) => Some(stream),
            Err(error) => {
                record_failure(failure, &error);
                None
            }
        }
    }

    async fn forward_responses(
        &self,
        mut responses: Box<dyn ResponseSource>,
        outbox: &mpsc::Sender<OutboxItem>,
        billed_before: f64,
        failure: &Mutex<Option<String>>,
    ) {
        loop {
            let Some(incoming) = self
                .halt
                .run_until_cancelled(responses.next_response())
                .await
            else {
                return;
            };
            match incoming {
                Some(Ok(response)) => self.emit(outbox, response, billed_before).await,
                Some(Err(error)) => {
                    record_failure(failure, &error);
                    return;
                }
                None => return,
            }
        }
    }

    async fn emit(
        &self,
        outbox: &mpsc::Sender<OutboxItem>,
        response: StreamingRecognizeResponse,
        billed_before: f64,
    ) {
        let activity = speech_activity(&response);
        if activity != SpeechActivity::None {
            self.speech_active
                .store(activity == SpeechActivity::Begin, Ordering::SeqCst);
        }
        let billed = billed_duration(&response);
        let cumulative = {
            let mut current = self.billed.lock().unwrap_or_else(PoisonError::into_inner);
            *current = (*current).max(billed);
            billed_before.mul_add(1.0, current.as_secs_f64())
        };
        let event = response_event(&response, cumulative);
        let result = OutboxItem::TurnResult {
            turn: self.turn,
            event,
        };
        let _sent = self.halt.run_until_cancelled(outbox.send(result)).await;
    }
}

struct Drain {
    queue: mpsc::Receiver<Option<StreamingRecognizeRequest>>,
    sink: Box<dyn RequestSink>,
    failure: Arc<Mutex<Option<String>>>,
    halt: CancellationToken,
}

impl Drain {
    async fn run(mut self) {
        while let Some(queued) = self.queue.recv().await {
            let Some(request) = queued else {
                return;
            };
            let sent = self.sink.send(request).await;
            if let Err(error) = sent {
                self.fail(&error);
                return;
            }
        }
    }

    fn fail(&self, error: &ChirpStreamError) {
        record_failure(&self.failure, error);
        self.halt.cancel();
    }
}

fn record_failure(slot: &Mutex<Option<String>>, error: &ChirpStreamError) {
    *slot.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(format!("Google Speech-to-Text streaming failed: {error}"));
}

async fn report_failure(outbox: &mpsc::Sender<OutboxItem>, failure: &Mutex<Option<String>>) {
    let pending = failure
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some(reason) = pending {
        tracing::warn!("{reason}");
        let _sent = outbox.send(OutboxItem::StreamFailure { reason }).await;
    }
}
