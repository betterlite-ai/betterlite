use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use betterlite_a2a::client::A2AClient;
use betterlite_a2a::send::asend_message_streaming;
use betterlite_a2a::send::call::StreamCall;
use betterlite_a2a::send::deps::{BridgeDeps, StreamingDeps};
use betterlite_a2a::send::error::SendEntryError;
use betterlite_a2a::streaming_iterator::DEFAULT_MAX_RETAINED_BYTES;
use betterlite_core::agents::json::AgentJson;
use betterlite_logging::logging_state::correlation::{CorrelationContext, CorrelationSettings};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use futures_util::StreamExt as _;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::{Mutex, Notify, mpsc};

use super::convert::Call;
use super::error::A2aCallError;
use super::payload::{entry_error, send_payload};
use super::ports::{NoCompletion, NoDispatch, NoProviders, SystemSource, Unavailable};
use super::setup::{Runtime, trace_ids};

const CHUNK_QUEUE: usize = 16;
type Chunk = Result<String, PyErr>;
type EntryError = SendEntryError<Unavailable, Unavailable>;
type Delivery = Result<(), SendError<Chunk>>;
type Driver = Pin<Box<dyn Future<Output = Delivery> + Send>>;

// One queued chunk plus the future still reading the agent; the future owns the sender and its connection.
struct Feed {
    receiver: mpsc::Receiver<Chunk>,
    driver: Option<Driver>,
}

impl Feed {
    // The caller holds the feed lock for one wait, so a close must not need that lock.
    async fn next_chunk(&mut self) -> Option<Chunk> {
        if let Some(mut driver) = self.driver.take() {
            tokio::select! {
                item = self.receiver.recv() => {
                    if item.is_some() {
                        self.driver = Some(driver);
                    }
                    return item;
                }
                _finished = &mut driver => {}
            }
        }
        self.receiver.recv().await
    }
}

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeA2aStream {
    feed: Arc<Mutex<Feed>>,
    cancel: Arc<Notify>,
    closed: Arc<AtomicBool>,
}

impl NativeA2aStream {
    fn new(receiver: mpsc::Receiver<Chunk>, driver: Driver) -> Self {
        Self {
            feed: Arc::new(Mutex::new(Feed {
                receiver,
                driver: Some(driver),
            })),
            cancel: Arc::new(Notify::new()),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    // aclose and a dropped handle both stop the producer through this one path.
    fn stop(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.cancel.notify_one();
        if let Ok(mut locked) = self.feed.try_lock() {
            locked.driver = None;
        }
    }
}

// The wait races the close signal, so aclose never queues behind a chunk that has not arrived.
async fn take_chunk(
    feed: Arc<Mutex<Feed>>,
    cancel: Arc<Notify>,
    closed: Arc<AtomicBool>,
) -> Option<Chunk> {
    if closed.load(Ordering::SeqCst) {
        return None;
    }
    let mut locked = feed.lock().await;
    if closed.load(Ordering::SeqCst) {
        locked.driver = None;
        return None;
    }
    let item = tokio::select! {
        item = locked.next_chunk() => item,
        () = cancel.notified() => None,
    };
    if closed.load(Ordering::SeqCst) {
        locked.driver = None;
        return None;
    }
    item
}

#[pymethods]
impl NativeA2aStream {
    fn anext_chunk<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let feed = Arc::clone(&self.feed);
        let cancel = Arc::clone(&self.cancel);
        let closed = Arc::clone(&self.closed);
        future_into_py(py, async move {
            match take_chunk(feed, cancel, closed).await {
                Some(Ok(text)) => Ok(Some(text)),
                Some(Err(error)) => Err(error),
                None => Ok(None),
            }
        })
    }

    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.stop();
        future_into_py(py, async { Ok::<(), PyErr>(()) })
    }
}

impl Drop for NativeA2aStream {
    fn drop(&mut self) {
        self.stop();
        if let Some(feed) = Arc::get_mut(&mut self.feed) {
            feed.get_mut().driver = None;
        }
    }
}

pub(crate) fn start(
    client: Option<A2AClient>,
    call: Call,
) -> Result<NativeA2aStream, A2aCallError> {
    let resolved = call.resolve(client)?;
    let wire = StreamCall {
        a2a_client: resolved.client,
        request: resolved.request,
        api_base: resolved.api_base,
        litellm_params: resolved.litellm_params,
        agent_id: resolved.agent_id,
        metadata: resolved.metadata,
        proxy_server_request: None,
        agent_extra_headers: resolved.agent_extra_headers,
    };
    let (sender, receiver) = mpsc::channel(CHUNK_QUEUE);
    let driver: Driver = Box::pin(produce(resolved.trace_id, wire, sender));
    Ok(NativeA2aStream::new(receiver, driver))
}

// The failure is queued after the chunks the agent already produced, and a closed consumer simply stops reading it.
async fn produce(
    trace_id: Option<String>,
    call: StreamCall,
    sender: mpsc::Sender<Chunk>,
) -> Delivery {
    let failure = match run(trace_id, call, &sender).await {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    sender.send(Err(failure)).await
}

// B4814's streaming entry runs here, one chunk at a time, until the agent's stream ends or fails.
async fn run(
    trace_id: Option<String>,
    call: StreamCall,
    sender: &mpsc::Sender<Chunk>,
) -> Result<(), PyErr> {
    let runtime = Runtime::build()?;
    let trace = trace_ids(trace_id);
    let start = Instant::now();
    let mut tick = move || start.elapsed();
    let mut correlation = CorrelationContext::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| PyErr::from(A2aCallError::Redactor { source }))?;
    let completion = NoCompletion;
    let providers = NoProviders;
    let mut source = SystemSource;
    let mut bridge = BridgeDeps {
        completion: &completion,
        providers: &providers,
        source: &mut source,
    };
    let deps: StreamingDeps<'_, _, NoDispatch> = StreamingDeps {
        native: runtime.native(&trace),
        clock: || Utc::now().fixed_offset(),
        monotonic: &mut tick,
        correlation: &mut correlation,
        correlation_settings: CorrelationSettings::default(),
        redactor: &redactor,
        dispatch: NoDispatch,
        max_retained_bytes: DEFAULT_MAX_RETAINED_BYTES,
    };
    let mut events = asend_message_streaming(call, None, deps, &mut bridge)
        .await
        .map_err(|error| send_payload(error, true).into_exception())?;
    while let Some(item) = events.next().await {
        let text = chunk_of(item)?;
        if sender.send(Ok(text)).await.is_err() {
            return Ok(());
        }
    }
    Ok(())
}

fn chunk_of(item: Result<AgentJson, EntryError>) -> Result<String, PyErr> {
    match item {
        Ok(value) => serde_json::to_string(&value)
            .map_err(|source| PyErr::from(A2aCallError::Encode { source })),
        Err(error) => Err(entry_error(error, true)),
    }
}
