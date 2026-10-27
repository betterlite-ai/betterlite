use crate::analysis::primitives::{concurrent_results, feedback_pages, read_execution};
use crate::analysis::{AnalysisError, Examined, ModelCall, ReadContent, ReportProgress};
use crate::models::execution::{Coverage, Execution, Sample};
use crate::models::messages::Claim;
use crate::trace_store::TraceStoreError;
use std::{num::TryFromIntError, sync::Arc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractionError {
    #[error("extraction failed: {0}")]
    Analysis(#[from] AnalysisError),
    #[error("extraction count exceeds platform limits: {0}")]
    Count(#[from] TryFromIntError),
}

mod evidence;
mod navigation;
mod prompt;
mod storage;
mod tests;

async fn extract(
    claim: Claim,
    execution: Execution,
    read: ReadContent,
    model: ModelCall,
) -> Result<Examined, AnalysisError> {
    let mut store = storage::open().await?;
    match extract_stored(&claim, &execution, &read, &model, &mut store).await {
        Ok(examined) => Ok(examined),
        Err(AnalysisError::Validation(_) | AnalysisError::Decode(_)) => Ok(Examined {
            execution,
            observations: Vec::new(),
            parts: Vec::new(),
            partial: true,
            cannot_assess: true,
        }),
        Err(error) => Err(error),
    }
}

async fn extract_stored(
    claim: &Claim,
    execution: &Execution,
    read: &ReadContent,
    model: &ModelCall,
    store: &mut storage::Stored,
) -> Result<Examined, AnalysisError> {
    let page = read_execution(
        execution.clone(),
        read,
        store.inner.as_mut().ok_or(TraceStoreError::Unavailable)?,
    )
    .await?;
    let summary = store.access(storage::summarize).await?;
    let context = prompt::Context {
        claim,
        execution,
        page: &page,
        summary: &summary,
        feedback: feedback_pages(claim, None)?,
    };
    let mut reviews = Vec::new();
    for catalog in &summary.catalogs {
        let response = navigation::examine(&context, catalog, store, read, model).await?;
        let checks = claim.job.settings.analysis_checks();
        let verified = store
            .access(move |stored| evidence::verify(response, stored, &checks))
            .await?;
        reviews.push(verified);
    }
    Ok(evidence::combine(execution, &reviews, &summary, &page))
}

pub async fn examine_executions(
    claim: Claim,
    sample: Sample,
    read: ReadContent,
    model: ModelCall,
    progress: ReportProgress,
) -> Result<Vec<Examined>, ExtractionError> {
    let mut coverage = Coverage {
        eligible: sample.eligible,
        selected: i64::try_from(sample.executions.len())?,
        screened: 0,
        investigated: 0,
        inconclusive: 0,
        grouping_batches: 0,
        grouped_batches: 0,
        candidates: 0,
        partial: 0,
        unassessable: 0,
    };
    progress(String::from("Reading executions"), coverage.clone()).await?;
    let concurrency = usize::try_from(claim.job.settings.concurrency)?;
    let mut results = concurrent_results(
        sample.executions,
        move |execution| {
            extract(
                claim.clone(),
                execution,
                Arc::clone(&read),
                Arc::clone(&model),
            )
        },
        Some(concurrency),
    )?;
    let mut examined = Vec::new();
    while let Some(result) = results.next().await {
        let item = result?;
        coverage.screened = coverage.screened.saturating_add(1);
        progress(String::from("Reading executions"), coverage.clone()).await?;
        examined.push(item);
    }
    Ok(examined)
}

#[path = "extraction/partial_status/tests.rs"]
mod partial_status;

#[path = "extraction/extract/tests.rs"]
mod extract;

#[path = "extraction/extract_stored/tests.rs"]
mod extract_stored;

#[path = "extraction/feedback_loop_detection/tests.rs"]
mod feedback_loop_detection;

#[path = "extraction/span_read_limit/tests.rs"]
mod span_read_limit;

#[path = "extraction/observation_filtering/tests.rs"]
mod observation_filtering;

#[path = "extraction/partial_coverage/tests.rs"]
mod partial_coverage;

#[path = "extraction/evidence_dedup/tests.rs"]
mod evidence_dedup;

#[path = "extraction/examine_executions/tests.rs"]
mod examine_executions;
#[path = "extraction/review_prompt/tests.rs"]
mod review_prompt;
