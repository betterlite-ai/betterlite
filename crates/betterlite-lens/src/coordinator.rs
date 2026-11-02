use crate::analysis::{AnalysisError, ModelCall, ReadContent, ReportProgress};
use crate::extraction::ExtractionError;
use crate::models::execution::{Execution, Sample};
use crate::models::messages::{Claim, JobResult};
use indexmap::IndexMap;
use std::num::TryFromIntError;
use thiserror::Error;

mod alias;
mod batches;
mod cluster;
mod merge;
mod orchestrate;
mod prompt;

#[derive(Debug, Error)]
pub enum CoordinatorError {
    #[error("analysis failed: {0}")]
    Analysis(#[from] AnalysisError),
    #[error("extraction failed: {0}")]
    Extraction(#[from] ExtractionError),
    #[error("sample size exceeds platform limits: {0}")]
    Count(#[from] TryFromIntError),
}

pub async fn analyze_sample(
    claim: Claim,
    sample: Sample,
    read: ReadContent,
    model: ModelCall,
    progress: ReportProgress,
) -> Result<JobResult, CoordinatorError> {
    let originals: IndexMap<String, Execution> = sample
        .executions
        .iter()
        .enumerate()
        .map(|(index, execution)| (format!("r{index}"), execution.clone()))
        .collect();
    let aliased_sample = alias::aliased_sample(sample, &originals);
    let aliased_read = alias::aliased_read(read, originals.clone());
    let result = orchestrate::run(claim, aliased_sample, aliased_read, model, progress).await?;
    Ok(alias::restore(result, &originals))
}

#[path = "coordinator/tests/fixtures.rs"]
mod fixtures;

#[path = "coordinator/tests/active_settled.rs"]
mod active_settled;
#[path = "coordinator/tests/analyze_sample.rs"]
mod analyze_sample;
#[path = "coordinator/tests/candidate_partitioning.rs"]
mod candidate_partitioning;
#[path = "coordinator/tests/candidate_size.rs"]
mod candidate_size;
#[path = "coordinator/tests/cluster_batches.rs"]
mod cluster_batches;
#[path = "coordinator/tests/concurrency.rs"]
mod concurrency;
#[path = "coordinator/tests/consolidate.rs"]
mod consolidate;
#[path = "coordinator/tests/duplicate_reference.rs"]
mod duplicate_reference;
#[path = "coordinator/tests/empty_executions.rs"]
mod empty_executions;
#[path = "coordinator/tests/empty_observations.rs"]
mod empty_observations;
#[path = "coordinator/tests/invalid_grouping.rs"]
mod invalid_grouping;
#[path = "coordinator/tests/merge_candidates.rs"]
mod merge_candidates;
#[path = "coordinator/tests/observation_batches.rs"]
mod observation_batches;
#[path = "coordinator/tests/progress_reporting.rs"]
mod progress_reporting;
