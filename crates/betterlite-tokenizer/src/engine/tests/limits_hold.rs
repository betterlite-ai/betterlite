use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use crate::engine::Engine;
use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use crate::engine::tiktoken::{Encoding, TiktokenEngine};

struct Recorder {
    running: AtomicUsize,
    peak: AtomicUsize,
    started: AtomicUsize,
}

impl Recorder {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            running: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl BlockingCounter for Recorder {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        let running = self
            .running
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.peak.fetch_max(running, Ordering::SeqCst);
        self.started.fetch_add(1, Ordering::SeqCst);
        sleep(Duration::from_millis(20)).await;
        self.running.fetch_sub(1, Ordering::SeqCst);
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

struct GatedCounter {
    entered: mpsc::Sender<()>,
    open: AtomicBool,
}

#[async_trait]
impl BlockingCounter for GatedCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        if self.open.load(Ordering::SeqCst) {
            return job();
        }
        let _sent = self.entered.send(()).await;
        pending().await
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

fn cl100k() -> Engine {
    let data = assets::bundled(Encoding::Cl100kBase).unwrap();
    Engine::Tiktoken(Arc::new(
        TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).unwrap(),
    ))
}

fn spawn_counts(
    service: &Arc<CountingService>,
    engine: &Engine,
    texts: &[&'static str],
) -> JoinSet<Result<usize, EngineError>> {
    let mut tasks = JoinSet::new();
    for &text in texts {
        let owned_service = Arc::clone(service);
        let owned_engine = engine.clone();
        tasks.spawn(async move { owned_service.count(&owned_engine, text).await });
    }
    tasks
}

#[rstest]
#[case::one_at_a_time(1)]
#[case::two_at_a_time(2)]
#[tokio::test]
async fn the_limiter_caps_concurrency(#[case] max_concurrent: usize) {
    let recorder = Recorder::new();
    let facility: Arc<dyn BlockingCounter> = Arc::<Recorder>::clone(&recorder);
    let service = Arc::new(CountingService::new(
        max_concurrent,
        facility,
        EncodeLimits::default(),
    ));
    let engine = cl100k();
    let mut tasks = spawn_counts(&service, &engine, &["hello world"; 6]);
    while let Some(result) = tasks.join_next().await {
        assert_eq!(result.unwrap().unwrap(), 2);
    }
    let peak = recorder.peak.load(Ordering::SeqCst);
    assert!(peak <= max_concurrent, "peak {peak} over {max_concurrent}");
    assert_eq!(recorder.started.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn the_input_queue_admits_every_caller() {
    let recorder = Recorder::new();
    let facility: Arc<dyn BlockingCounter> = Arc::<Recorder>::clone(&recorder);
    let service = Arc::new(CountingService::new(2, facility, EncodeLimits::default()));
    let engine = cl100k();
    let mut tasks = spawn_counts(&service, &engine, &["a"; 8]);
    while let Some(result) = tasks.join_next().await {
        assert_eq!(result.unwrap().unwrap(), 1);
    }
    assert_eq!(recorder.peak.load(Ordering::SeqCst), 2);
    assert_eq!(recorder.started.load(Ordering::SeqCst), 8);
}

#[tokio::test]
async fn every_caller_receives_its_own_output() {
    let recorder = Recorder::new();
    let service = Arc::new(CountingService::new(2, recorder, EncodeLimits::default()));
    let engine = cl100k();
    let mut tasks = spawn_counts(&service, &engine, &["hello", "hello world", "", "a   b"]);
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap().unwrap());
    }
    results.sort_unstable();
    assert_eq!(results, [0, 1, 2, 3]);
}

#[tokio::test]
async fn a_cancelled_count_releases_its_permit() {
    let (sender, mut entered) = mpsc::channel(1);
    let counter = Arc::new(GatedCounter {
        entered: sender,
        open: AtomicBool::new(false),
    });
    let facility: Arc<dyn BlockingCounter> = Arc::<GatedCounter>::clone(&counter);
    let service = Arc::new(CountingService::new(1, facility, EncodeLimits::default()));
    let engine = cl100k();
    let mut tasks = JoinSet::new();
    let first = Arc::clone(&service);
    let first_engine = engine.clone();
    tasks.spawn(async move { first.count(&first_engine, "hello").await });
    entered.recv().await.unwrap();
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    counter.open.store(true, Ordering::SeqCst);
    let outcome = timeout(
        Duration::from_secs(5),
        service.count(&engine, "hello world"),
    )
    .await;
    assert_eq!(outcome.unwrap().unwrap(), 2);
}
#[tokio::test]
async fn the_input_cap_rejects_long_text_before_admitting_it() {
    let recorder = Recorder::new();
    let facility: Arc<dyn BlockingCounter> = Arc::<Recorder>::clone(&recorder);
    let limits = EncodeLimits {
        input_bytes: 4,
        output_ids: 100,
    };
    let service = Arc::new(CountingService::new(1, facility, limits));
    let engine = cl100k();
    assert!(matches!(
        service.count(&engine, "hello world").await,
        Err(EngineError::InputTooLarge {
            bytes: 11,
            limit: 4
        })
    ));
    assert_eq!(recorder.started.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_output_cap_rejects_long_id_lists() {
    let recorder = Recorder::new();
    let facility: Arc<dyn BlockingCounter> = Arc::<Recorder>::clone(&recorder);
    let limits = EncodeLimits {
        input_bytes: 100,
        output_ids: 1,
    };
    let service = Arc::new(CountingService::new(1, facility, limits));
    let engine = cl100k();
    assert!(matches!(
        service.count(&engine, "hello world").await,
        Err(EngineError::OutputTooLarge { ids: 2, limit: 1 })
    ));
}
