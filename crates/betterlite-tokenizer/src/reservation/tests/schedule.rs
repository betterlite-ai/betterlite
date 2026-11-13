use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::count::Counter;
use crate::engine::Engine;
use crate::engine::error::EngineError;
use crate::engine::limits::{BlockingCounter, BuildJob, ConvertJob, CountJob};
use crate::reservation::error::ReservationError;
use crate::reservation::schedule::{AccountJob, AccountPool, BoundedPool};
use crate::reservation::{
    ParsedRequest, RequestBody, approximate_input_size, count_input_tokens,
    count_input_tokens_for_model,
};

use super::support;

const MODEL: &str = "gpt-3.5-turbo";

struct RecordingPool {
    runs: Arc<AtomicUsize>,
}

#[async_trait]
impl AccountPool for RecordingPool {
    async fn run(&self, job: AccountJob) -> Result<IndexMap<String, usize>, ReservationError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(job.await)
    }
}

struct FailingPool;

#[async_trait]
impl AccountPool for FailingPool {
    async fn run(&self, _job: AccountJob) -> Result<IndexMap<String, usize>, ReservationError> {
        Err(ReservationError::PoolClosed)
    }
}

struct GatedFacility {
    entered: mpsc::Sender<()>,
    calls: AtomicUsize,
}

#[async_trait]
impl BlockingCounter for GatedFacility {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let _sent = self.entered.send(()).await;
            return pending().await;
        }
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

fn prompt_body(length: usize) -> RequestBody {
    let mut body = RequestBody::new();
    body.insert("prompt".to_owned(), Json::Text("a".repeat(length)));
    body
}

fn models() -> Vec<String> {
    vec![MODEL.to_owned()]
}

#[tokio::test]
async fn the_offload_starts_at_thirty_thousand_characters() {
    let directory = tempfile::tempdir().unwrap();
    let counter = Arc::new(support::legacy_counter(directory.path()));
    let runs = Arc::new(AtomicUsize::new(0));
    let pool = RecordingPool {
        runs: Arc::clone(&runs),
    };
    let models = models();
    let under = prompt_body(29_999);
    let at = prompt_body(30_000);
    assert_eq!(approximate_input_size(&under), 29_999);
    assert_eq!(approximate_input_size(&at), 30_000);
    let inline = count_input_tokens(
        &counter,
        ParsedRequest {
            body: &under,
            raw_body: None,
        },
        &models,
        &pool,
    )
    .await;
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    let offloaded = count_input_tokens(
        &counter,
        ParsedRequest {
            body: &at,
            raw_body: None,
        },
        &models,
        &pool,
    )
    .await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let direct = count_input_tokens_for_model(counter.as_ref(), &at, MODEL).await;
    assert_eq!(offloaded.get(MODEL), direct.as_ref());
    assert_eq!(inline.len(), 1);
}

#[tokio::test]
async fn a_pool_failure_counts_inline_instead() {
    let directory = tempfile::tempdir().unwrap();
    let counter = Arc::new(support::legacy_counter(directory.path()));
    let body = prompt_body(30_000);
    let counts = count_input_tokens(
        &counter,
        ParsedRequest {
            body: &body,
            raw_body: None,
        },
        &models(),
        &FailingPool,
    )
    .await;
    let direct = count_input_tokens_for_model(counter.as_ref(), &body, MODEL).await;
    assert_eq!(counts.get(MODEL), direct.as_ref());
}

#[tokio::test]
async fn duplicate_models_keep_the_first_position_and_count_their_own_model() {
    let directory = tempfile::tempdir().unwrap();
    let counter = Arc::new(support::legacy_counter(directory.path()));
    let body = support::body(
        r#"{"messages": [{"role": "user", "content": "hello world", "name": "bob"}]}"#,
    );
    let models = vec![
        "gpt-3.5-turbo-0301".to_owned(),
        MODEL.to_owned(),
        "gpt-3.5-turbo-0301".to_owned(),
    ];
    let counts = count_input_tokens(
        &counter,
        ParsedRequest {
            body: &body,
            raw_body: None,
        },
        &models,
        &FailingPool,
    )
    .await;
    assert_eq!(
        counts.keys().map(String::as_str).collect::<Vec<_>>(),
        ["gpt-3.5-turbo-0301", MODEL]
    );
    // 4 per message + role(1) + content(2) + name(1) - 1, against 3 per message + name overhead(1)
    assert_eq!(counts.get("gpt-3.5-turbo-0301"), Some(&10));
    assert_eq!(counts.get(MODEL), Some(&11));
}

#[tokio::test]
async fn raw_bytes_do_not_change_a_count_under_the_python_rollout() {
    let directory = tempfile::tempdir().unwrap();
    let counter = Arc::new(support::legacy_counter(directory.path()));
    let body = support::body(r#"{"prompt": "hello world"}"#);
    let models = models();
    let with_bytes = count_input_tokens(
        &counter,
        ParsedRequest {
            body: &body,
            raw_body: Some(b"{\"prompt\": \"other\"}"),
        },
        &models,
        &FailingPool,
    )
    .await;
    let plain = count_input_tokens(
        &counter,
        ParsedRequest {
            body: &body,
            raw_body: None,
        },
        &models,
        &FailingPool,
    )
    .await;
    assert_eq!(with_bytes, plain);
    assert_eq!(plain.get(MODEL), Some(&2));
}

#[tokio::test]
async fn mixed_models_leave_out_the_model_whose_count_fails() {
    let directory = tempfile::tempdir().unwrap();
    let counter = Arc::new(support::dead_remote_counter(directory.path()));
    let body = support::body(r#"{"prompt": "hello world"}"#);
    let models = vec![MODEL.to_owned(), "text-davinci-001".to_owned()];
    let counts = count_input_tokens(
        &counter,
        ParsedRequest {
            body: &body,
            raw_body: None,
        },
        &models,
        &FailingPool,
    )
    .await;
    assert_eq!(counts.len(), 1);
    assert_eq!(counts.get(MODEL), Some(&2));
}

#[tokio::test]
async fn a_cancelled_offload_frees_the_pool_for_the_next_request() {
    let (_directory, counter, pool, mut entered) = gated_fixture();
    let models = models();
    let big = prompt_body(30_000);
    let small = prompt_body(5);
    let mut tasks = JoinSet::new();
    spawn_count(&mut tasks, &counter, &pool, big.clone(), models.clone());
    let admitted = timeout(Duration::from_secs(5), entered.recv()).await;
    assert!(matches!(admitted, Ok(Some(()))));
    let small_counts = count_with(&counter, &pool, &small, &models).await;
    assert_eq!(small_counts.len(), 1);
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let retried = timeout(
        Duration::from_secs(5),
        count_with(&counter, &pool, &big, &models),
    )
    .await;
    assert!(matches!(retried, Ok(counts) if counts.len() == 1));
}

fn gated_fixture() -> (
    tempfile::TempDir,
    Arc<Counter>,
    Arc<BoundedPool>,
    mpsc::Receiver<()>,
) {
    let directory = tempfile::tempdir().unwrap();
    let (sender, entered) = mpsc::channel(1);
    let facility = Arc::new(GatedFacility {
        entered: sender,
        calls: AtomicUsize::new(0),
    });
    let counter = Arc::new(support::counter_with_facility(
        directory.path(),
        facility,
        2,
    ));
    let pool = Arc::new(BoundedPool::new(1));
    (directory, counter, pool, entered)
}

fn spawn_count(
    tasks: &mut JoinSet<IndexMap<String, usize>>,
    counter: &Arc<Counter>,
    pool: &Arc<BoundedPool>,
    body: RequestBody,
    models: Vec<String>,
) {
    let task_counter = Arc::clone(counter);
    let task_pool = Arc::clone(pool);
    tasks.spawn(async move {
        count_input_tokens(
            &task_counter,
            ParsedRequest {
                body: &body,
                raw_body: None,
            },
            &models,
            task_pool.as_ref(),
        )
        .await
    });
}

async fn count_with(
    counter: &Arc<Counter>,
    pool: &Arc<BoundedPool>,
    body: &RequestBody,
    models: &[String],
) -> IndexMap<String, usize> {
    count_input_tokens(
        counter,
        ParsedRequest {
            body,
            raw_body: None,
        },
        models,
        pool.as_ref(),
    )
    .await
}
