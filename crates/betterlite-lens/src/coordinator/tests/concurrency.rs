#![cfg(test)]
use super::fixtures::{
    capture_progress, claim, execution, extraction_observation, inconclusive_decision, page, part,
    pipeline_model, reader,
};
use super::orchestrate::{limited, run};
use crate::analysis::{AnalysisError, ModelCall};
use crate::models::execution::Sample;
use crate::models::messages::{ModelRequest, ModelResult, Purpose};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use tokio::task::{JoinSet, yield_now};
use tokio::time::{sleep, timeout};

fn gated_model(entered: Arc<AtomicUsize>, release: Arc<Notify>) -> ModelCall {
    Arc::new(move |_| {
        let counter = Arc::clone(&entered);
        let gate = Arc::clone(&release);
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            gate.notified().await;
            Ok(ModelResult {
                content: String::from("{}"),
                cost: rust_decimal::Decimal::ZERO,
            })
        })
    })
}

fn spawn_call(
    tasks: &mut JoinSet<Result<ModelResult, AnalysisError>>,
    model: &ModelCall,
    request: &ModelRequest,
) {
    let callee = Arc::clone(model);
    let call = request.clone();
    tasks.spawn(async move { callee(call).await });
}

#[tokio::test]
async fn blocks_the_second_call_until_the_first_releases_its_permit() {
    let semaphore = Arc::new(Semaphore::new(1));
    let release = Arc::new(Notify::new());
    let entered = Arc::new(AtomicUsize::new(0));
    let limited_model = limited(
        semaphore,
        gated_model(Arc::clone(&entered), Arc::clone(&release)),
    );
    let request = ModelRequest {
        prompt: String::from("x"),
        purpose: Purpose::Cluster,
    };
    let mut tasks = JoinSet::new();
    spawn_call(&mut tasks, &limited_model, &request);
    while entered.load(Ordering::SeqCst) < 1 {
        yield_now().await;
    }
    spawn_call(&mut tasks, &limited_model, &request);
    sleep(Duration::from_millis(20)).await;
    assert_eq!(entered.load(Ordering::SeqCst), 1);
    release.notify_one();
    release.notify_one();
    timeout(Duration::from_secs(1), async {
        while let Some(outcome) = tasks.join_next().await {
            outcome.unwrap().unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(entered.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn one_semaphore_is_shared_across_extraction_and_clustering_without_deadlock() {
    let mut input = claim();
    input.job.settings.concurrency = 1;
    let (progress, _log) = capture_progress();
    let sample = Sample {
        executions: vec![execution("r0"), execution("r1")],
        eligible: 2,
        selected: 2,
        next_offset: None,
        next_cursor: None,
    };
    let pages = vec![
        page(part("r0", "s0", "first")),
        page(part("r1", "s1", "second")),
    ];
    let extraction = vec![
        extraction_observation("r0", "s0", "expected_behavior", "first"),
        extraction_observation("r1", "s1", "expected_behavior", "second"),
    ];
    let investigation = vec![inconclusive_decision(), inconclusive_decision()];
    let model_call = pipeline_model(extraction, investigation);
    let result = timeout(
        Duration::from_secs(2),
        run(input, sample, reader(pages), model_call, progress),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result.findings.is_empty());
    assert_eq!(result.coverage.investigated, 2);
}
