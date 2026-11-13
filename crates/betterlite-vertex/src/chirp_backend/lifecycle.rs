use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use google_cloud_speech_v2::model::StreamingRecognitionConfig;
use secrecy::SecretString;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::chirp_backend::commands::{ChirpCommand, ChirpEvent, decode_chirp_command};
use crate::chirp_backend::error::ChirpBackendError;
use crate::chirp_backend::policy::{self, Delivery, NORMAL_CLOSURE_CODE, RotationLimits};
use crate::chirp_backend::pump::{Link, run_pump};
use crate::chirp_stream::client::{ClientFuture, SpeechStreamingClient};
use crate::chirp_stream::error::ChirpStreamError;
use crate::chirp_stream::lifecycle::RecognizeStream;
use crate::chirp_stream::limits::OUTBOX_SIZE;
use crate::chirp_stream::outbox::OutboxItem;
use crate::chirp_stream::records::{StreamingConfigure, first_request, streaming_config};
use crate::chirp_stream::target::SpeechStreamingTarget;

mod tests;

const LINK_QUEUE_SIZE: usize = 64;

pub trait ChirpClock: Send + Sync {
    fn now(&self) -> Instant;
}

pub trait SpeechClientFactory: Send + Sync {
    fn open<'client>(
        &'client self,
        target: &'client SpeechStreamingTarget,
        token: SecretString,
    ) -> ClientFuture<'client, Result<Arc<dyn SpeechStreamingClient>, ChirpStreamError>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChirpMessage {
    Text(String),
    Audio(Vec<u8>),
}

pub struct ChirpBackend {
    target: SpeechStreamingTarget,
    clients: Arc<dyn SpeechClientFactory>,
    clock: Arc<dyn ChirpClock>,
    limits: RotationLimits,
    links: mpsc::Sender<Link>,
    link_queue: Option<mpsc::Receiver<Link>>,
    outbox: mpsc::Receiver<OutboxItem>,
    outbox_sender: mpsc::Sender<OutboxItem>,
    pump: JoinSet<Result<(), serde_json::Error>>,
    shutdown: CancellationToken,
    config: Option<StreamingRecognitionConfig>,
    turn: Vec<Arc<RecognizeStream>>,
    turn_index: usize,
    discarded: BTreeSet<usize>,
    closed: bool,
}

impl ChirpBackend {
    #[must_use]
    pub fn new(
        target: SpeechStreamingTarget,
        clients: Arc<dyn SpeechClientFactory>,
        clock: Arc<dyn ChirpClock>,
    ) -> Self {
        let (outbox_sender, outbox) = mpsc::channel(OUTBOX_SIZE);
        let (links, link_queue) = mpsc::channel(LINK_QUEUE_SIZE);
        Self {
            target,
            clients,
            clock,
            limits: RotationLimits::default(),
            links,
            link_queue: Some(link_queue),
            outbox,
            outbox_sender,
            pump: JoinSet::new(),
            shutdown: CancellationToken::new(),
            config: None,
            turn: Vec::new(),
            turn_index: 0,
            discarded: BTreeSet::new(),
            closed: false,
        }
    }

    pub async fn send(&mut self, message: ChirpMessage) -> Result<(), ChirpBackendError> {
        if self.closed {
            return Err(ChirpBackendError::Closed {
                code: NORMAL_CLOSURE_CODE,
                reason: String::new(),
            });
        }
        match message {
            ChirpMessage::Audio(audio) => {
                let stream = self.turn_stream().await?;
                stream.send_audio(audio).await?;
                Ok(())
            }
            ChirpMessage::Text(text) => {
                let command =
                    decode_chirp_command(text.as_bytes()).map_err(ChirpBackendError::Command)?;
                self.send_command(command).await
            }
        }
    }

    pub async fn recv(&mut self) -> Result<String, ChirpBackendError> {
        loop {
            if self.closed && self.outbox.is_empty() {
                return Err(ChirpBackendError::Closed {
                    code: NORMAL_CLOSURE_CODE,
                    reason: String::new(),
                });
            }
            let Some(item) = self.outbox.recv().await else {
                return Err(ChirpBackendError::QueueClosed);
            };
            match self.next_delivery(item)? {
                Delivery::Delivered(event) => return Ok(event),
                Delivery::Suppressed => {}
                Delivery::Close(frame) => {
                    return Err(ChirpBackendError::Closed {
                        code: frame.code,
                        reason: frame.reason,
                    });
                }
            }
        }
    }

    pub async fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.turn.clear();
        self.shutdown.cancel();
        while let Some(joined) = self.pump.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the Speech-to-Text link pump stopped");
                }
                Err(error) => tracing::warn!(%error, "the Speech-to-Text link pump failed"),
            }
        }
        if self.outbox.capacity() > 0 {
            drop(self.outbox_sender.try_send(OutboxItem::Closed));
        }
    }

    async fn send_command(&mut self, command: ChirpCommand) -> Result<(), ChirpBackendError> {
        match command {
            ChirpCommand::Configure {
                model,
                language_codes,
                sample_rate_hertz,
            } => {
                let configure = StreamingConfigure {
                    model,
                    language_codes,
                    sample_rate_hertz,
                };
                self.config = Some(streaming_config(&configure));
                let event = serde_json::to_string(&ChirpEvent::Configured)
                    .map_err(ChirpBackendError::Encode)?;
                self.link(Link::Event(event)).await
            }
            ChirpCommand::FinishTurn {} => self.finish_turn().await,
            ChirpCommand::DiscardTurn {} => self.discard_turn().await,
        }
    }

    async fn turn_stream(&mut self) -> Result<Arc<RecognizeStream>, ChirpBackendError> {
        let current = self.turn.last().map(Arc::clone);
        if let Some(stream) = current {
            let elapsed = self.clock.now().saturating_duration_since(stream.opened_at);
            let active = stream
                .speech_active
                .load(std::sync::atomic::Ordering::SeqCst);
            if !policy::rotation_due(elapsed, active, self.limits) {
                return Ok(stream);
            }
            stream.half_close().await?;
        }
        let stream = self.open_stream().await?;
        self.turn.push(Arc::clone(&stream));
        Ok(stream)
    }

    async fn open_stream(&mut self) -> Result<Arc<RecognizeStream>, ChirpBackendError> {
        let Some(config) = self.config.clone() else {
            return Err(ChirpBackendError::NotConfigured);
        };
        let token = self.target.token.resolve_access_token().await?;
        let client = self.clients.open(&self.target, token).await?;
        let request = first_request(&self.target.recognizer, config);
        let stream = Arc::new(RecognizeStream::new(
            client,
            request,
            self.clock.now(),
            self.turn_index,
        )?);
        self.link(Link::Stream(Arc::clone(&stream))).await?;
        Ok(stream)
    }

    async fn finish_turn(&mut self) -> Result<(), ChirpBackendError> {
        let turn = std::mem::take(&mut self.turn);
        self.turn_index = self.turn_index.saturating_add(1);
        if let Some(last) = turn.last() {
            last.half_close().await?;
        }
        let event =
            serde_json::to_string(&ChirpEvent::TurnFinished).map_err(ChirpBackendError::Encode)?;
        self.link(Link::Event(event)).await
    }

    async fn discard_turn(&mut self) -> Result<(), ChirpBackendError> {
        let turn = self.turn_index;
        let streams = std::mem::take(&mut self.turn);
        let _inserted = self.discarded.insert(turn);
        self.turn_index = self.turn_index.saturating_add(1);
        for stream in &streams {
            stream.halt.cancel();
        }
        self.link(Link::TurnDiscarded(turn)).await
    }

    async fn link(&mut self, link: Link) -> Result<(), ChirpBackendError> {
        if let Some(receiver) = self.link_queue.take() {
            let _pump = self.pump.spawn(run_pump(
                receiver,
                self.outbox_sender.clone(),
                self.shutdown.clone(),
            ));
        }
        self.links
            .send(link)
            .await
            .map_err(|_source| ChirpBackendError::QueueClosed)
    }

    fn next_delivery(&mut self, item: OutboxItem) -> Result<Delivery, ChirpBackendError> {
        let delivery = match item {
            OutboxItem::Text(event) => {
                policy::deliver(policy::Deliverable::Event(&event), &mut self.discarded)
            }
            OutboxItem::TurnResult { turn, event } => {
                let event = serde_json::to_string(&event).map_err(ChirpBackendError::Encode)?;
                policy::deliver(
                    policy::Deliverable::TurnResult {
                        turn,
                        event: &event,
                    },
                    &mut self.discarded,
                )
            }
            OutboxItem::TurnDiscardedEvent { turn, event } => policy::deliver(
                policy::Deliverable::DiscardMarker {
                    turn,
                    event: &event,
                },
                &mut self.discarded,
            ),
            OutboxItem::StreamFailure { reason } => policy::deliver(
                policy::Deliverable::StreamFailure { reason: &reason },
                &mut self.discarded,
            ),
            OutboxItem::Closed => policy::deliver(policy::Deliverable::Closed, &mut self.discarded),
        };
        Ok(delivery)
    }
}
