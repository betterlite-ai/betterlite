use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use google_cloud_speech_v2::model::{StreamingRecognizeRequest, StreamingRecognizeResponse};
use jiff::SignedDuration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::client::{RequestSink, ResponseSource, SpeechStreamingClient};
use super::error::ChirpStreamError;
use super::limits::REQUEST_QUEUE_SIZE;
use super::outbox::OutboxItem;
use super::records::{
    SpeechActivity, billed_duration, cumulative_billed, response_event, speech_activity,
};

mod tests;

#[derive(Clone)]
pub struct Connection {
    client: Arc<dyn SpeechStreamingClient>,
    closed: Arc<AtomicBool>,
}

impl Connection {
    pub fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.client.close();
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}

#[expect(
    clippy::partial_pub_fields,
    reason = "the consumer rotates on the open time, turn and activity, while the queue, billing and cancellation state stay internal"
)]
pub struct RecognizeStream {
    pub connection: Connection,
    pub opened_at: Instant,
    pub turn: usize,
    pub speech_active: Arc<AtomicBool>,
    requests: mpsc::Sender<Option<StreamingRecognizeRequest>>,
    receiver: Mutex<Option<mpsc::Receiver<Option<StreamingRecognizeRequest>>>>,
    billed: Arc<Mutex<SignedDuration>>,
    cancelled: Arc<AtomicBool>,
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
                closed: Arc::new(AtomicBool::new(false)),
            },
            opened_at,
            turn,
            speech_active: Arc::new(AtomicBool::new(false)),
            requests,
            receiver: Mutex::new(Some(receiver)),
            billed: Arc::new(Mutex::new(SignedDuration::ZERO)),
            cancelled: Arc::new(AtomicBool::new(false)),
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

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.connection.close();
    }

    pub async fn relay(
        &self,
        outbox: mpsc::Sender<OutboxItem>,
        billed_before: f64,
    ) -> SignedDuration {
        if self.cancelled.load(Ordering::SeqCst) {
            self.connection.close();
            return SignedDuration::ZERO;
        }
        let _closer = self.connection.clone();
        let Some(queue) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return *self.billed.lock().unwrap_or_else(PoisonError::into_inner);
        };
        let failed = Arc::new(AtomicBool::new(false));
        let mut tasks = ForwardTasks(JoinSet::new());
        let stream = match self.connection.client.streaming_recognize() {
            Ok(stream) => stream,
            Err(error) => {
                if !self.cancelled.load(Ordering::SeqCst) {
                    warn_and_enqueue(&outbox, error).await;
                }
                return *self.billed.lock().unwrap_or_else(PoisonError::into_inner);
            }
        };
        tasks.0.spawn(
            Drain {
                queue,
                sink: stream.requests,
                outbox: outbox.clone(),
                failed: Arc::clone(&failed),
                cancelled: Arc::clone(&self.cancelled),
                connection: self.connection.clone(),
            }
            .run(),
        );
        self.forward_responses(stream.responses, &outbox, billed_before, &failed)
            .await;
        *self.billed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn forward_responses(
        &self,
        mut responses: Box<dyn ResponseSource>,
        outbox: &mpsc::Sender<OutboxItem>,
        billed_before: f64,
        failed: &AtomicBool,
    ) {
        while let Some(incoming) = responses.next_response().await {
            match incoming {
                Ok(response) => self.emit(outbox, response, billed_before).await,
                Err(error)
                    if !self.cancelled.load(Ordering::SeqCst)
                        && !failed.swap(true, Ordering::SeqCst) =>
                {
                    warn_and_enqueue(outbox, error).await;
                    break;
                }
                Err(_) => break,
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
            cumulative_billed(billed_before, *current)
        };
        let event = response_event(&response, cumulative);
        let _sent = outbox
            .send(OutboxItem::TurnResult {
                turn: self.turn,
                event,
            })
            .await;
    }
}

struct ForwardTasks(JoinSet<()>);

impl Drop for ForwardTasks {
    fn drop(&mut self) {
        self.0.abort_all();
    }
}

struct Drain {
    queue: mpsc::Receiver<Option<StreamingRecognizeRequest>>,
    sink: Box<dyn RequestSink>,
    outbox: mpsc::Sender<OutboxItem>,
    failed: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    connection: Connection,
}

impl Drain {
    async fn run(mut self) {
        while let Some(queued) = self.queue.recv().await {
            let Some(request) = queued else {
                return;
            };
            let sent = self.sink.send(request).await;
            if let Err(error) = sent {
                self.fail(error).await;
                return;
            }
        }
    }

    async fn fail(&self, error: ChirpStreamError) {
        if self.cancelled.load(Ordering::SeqCst) || self.failed.swap(true, Ordering::SeqCst) {
            return;
        }
        warn_and_enqueue(&self.outbox, error).await;
        self.connection.close();
    }
}

async fn warn_and_enqueue(outbox: &mpsc::Sender<OutboxItem>, error: ChirpStreamError) {
    let reason = format!("Google Speech-to-Text streaming failed: {error}");
    tracing::warn!("{reason}");
    let _sent = outbox.send(OutboxItem::StreamFailure { reason }).await;
}
