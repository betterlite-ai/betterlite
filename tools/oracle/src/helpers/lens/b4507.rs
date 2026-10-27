use crate::helpers::{HelperError, HelperResult};
use betterlite_lens::analysis::{AnalysisError, ModelCall, ReadContent, ReportProgress};
use betterlite_lens::extraction::examine_executions;
use betterlite_lens::models::execution::{Coverage, ExecutionContent, Sample};
use betterlite_lens::models::messages::{Claim, ModelResult};
use serde::Deserialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[derive(Clone, Deserialize)]
pub(crate) struct Input {
    claim: Claim,
    sample: Sample,
    pages: Vec<Page>,
    responses: Vec<ModelResult>,
}
#[derive(Clone, Deserialize)]
struct Page {
    cursor: String,
    offset: i64,
    content: ExecutionContent,
}

pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(failure)?;
        runtime.block_on(run(input.clone()))
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(HelperError {
        class: String::from("ExtractionError"),
        message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

async fn run(input: Input) -> HelperResult {
    let concurrency = usize::try_from(input.claim.job.settings.concurrency).map_err(failure)?;
    let progress_log = Arc::new(Mutex::new(Vec::<Coverage>::new()));
    let counters = Arc::new(Counters::default());
    let progress_copy = Arc::clone(&progress_log);
    let progress: ReportProgress = Arc::new(move |_, coverage| {
        let result = progress_copy
            .lock()
            .map(|mut entries| entries.push(coverage))
            .map_err(|error| AnalysisError::Callback(error.to_string()));
        Box::pin(async move { result })
    });
    let (read, model) = callbacks(input.pages, input.responses, Arc::clone(&counters));
    let mut examined = examine_executions(input.claim, input.sample, read, model, progress)
        .await
        .map_err(failure)?;
    examined.sort_by(|left, right| left.execution.id.cmp(&right.execution.id));
    let coverage = progress_log.lock().map_err(failure)?.clone();
    serde_json::to_value((
        examined,
        coverage,
        counters.reads.load(Ordering::SeqCst),
        counters.models.load(Ordering::SeqCst),
        counters.maximum.load(Ordering::SeqCst) <= concurrency,
    ))
    .map_err(failure)
}

#[derive(Default)]
struct Counters {
    reads: AtomicUsize,
    models: AtomicUsize,
    active: AtomicUsize,
    maximum: AtomicUsize,
}

fn callbacks(
    pages: Vec<Page>,
    responses: Vec<ModelResult>,
    counters: Arc<Counters>,
) -> (ReadContent, ModelCall) {
    let model_counters = Arc::clone(&counters);
    let read: ReadContent = Arc::new(move |execution, cursor, offset| {
        counters.reads.fetch_add(1, Ordering::SeqCst);
        let content = pages
            .iter()
            .find(|page| {
                page.content.execution.id == execution
                    && page.cursor == cursor
                    && page.offset == offset
            })
            .map(|page| page.content.clone());
        Box::pin(async move {
            content.ok_or_else(|| AnalysisError::Callback(String::from("unscripted read")))
        })
    });
    let model: ModelCall = Arc::new(move |_| {
        let index = model_counters.models.fetch_add(1, Ordering::SeqCst);
        let result = responses.get(index).cloned();
        let current = Arc::clone(&model_counters);
        Box::pin(async move {
            let active = current
                .active
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1);
            current.maximum.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(10)).await;
            current.active.fetch_sub(1, Ordering::SeqCst);
            result.ok_or_else(|| AnalysisError::Callback(String::from("unscripted model call")))
        })
    });
    (read, model)
}
