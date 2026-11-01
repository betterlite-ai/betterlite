use super::CoordinatorError;
use super::batches::observation_batches;
use super::cluster::cluster_batches;
use crate::analysis::{
    AnalysisError, Examined, ModelCall, ModelFuture, Observation, ObservationKind, ReadContent,
    ReportProgress,
};
use crate::extraction::examine_executions;
use crate::investigation::coordinator::{Context, Investigations, investigate_candidates};
use crate::models::execution::{Coverage, RunAssessment, Sample};
use crate::models::finding::FindingDraft;
use crate::models::messages::{Claim, JobResult, ModelRequest};
use std::sync::Arc;
use tokio::sync::Semaphore;

struct Channels {
    claim: Claim,
    read: ReadContent,
    model: ModelCall,
    progress: ReportProgress,
}

struct Screened {
    examined: Vec<Examined>,
    coverage: Coverage,
    assessments: Vec<RunAssessment>,
}

pub(super) async fn run(
    claim: Claim,
    sample: Sample,
    read: ReadContent,
    model: ModelCall,
    progress: ReportProgress,
) -> Result<JobResult, CoordinatorError> {
    let base = base_coverage(&sample)?;
    if sample.executions.is_empty() {
        return Ok(JobResult {
            coverage: base,
            assessments: Vec::new(),
            findings: Vec::new(),
            error: String::new(),
        });
    }
    let max_concurrency = usize::try_from(claim.job.settings.concurrency).map_err(|error| {
        CoordinatorError::Analysis(AnalysisError::Validation(format!(
            "concurrency is out of range: {error}"
        )))
    })?;
    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    let limited_model = limited(semaphore, model);
    let channels = Channels {
        claim,
        read,
        model: limited_model,
        progress,
    };
    let screened = screen(&channels, sample, base).await?;
    let observations: Vec<Observation> = screened
        .examined
        .iter()
        .flat_map(|item| item.observations.clone())
        .collect();
    if observations.is_empty() {
        return Ok(JobResult {
            coverage: screened.coverage,
            assessments: screened.assessments,
            findings: Vec::new(),
            error: String::new(),
        });
    }
    group_and_investigate(channels, screened, observations).await
}

async fn screen(
    channels: &Channels,
    sample: Sample,
    base: Coverage,
) -> Result<Screened, CoordinatorError> {
    let examined = examine_executions(
        channels.claim.clone(),
        sample,
        Arc::clone(&channels.read),
        Arc::clone(&channels.model),
        Arc::clone(&channels.progress),
    )
    .await?;
    let coverage = screened_coverage(&base, &examined)?;
    let assessments: Vec<RunAssessment> = examined
        .iter()
        .map(|item| RunAssessment {
            execution_id: item.execution.id.clone(),
            issue_checks: unique_checks(&item.observations, ObservationKind::Issue),
            pattern_checks: unique_checks(&item.observations, ObservationKind::Pattern),
            cannot_assess: item.cannot_assess,
        })
        .collect();
    (channels.progress)(String::from("Grouping observations"), coverage.clone()).await?;
    Ok(Screened {
        examined,
        coverage,
        assessments,
    })
}

async fn group_and_investigate(
    channels: Channels,
    screened: Screened,
    observations: Vec<Observation>,
) -> Result<JobResult, CoordinatorError> {
    let batches = observation_batches(&observations)?;
    let batch_count = batches.len();
    let grouping = grouping_coverage(&screened.coverage, batch_count)?;
    let clusters = cluster_batches(
        batches,
        Arc::clone(&channels.model),
        Arc::clone(&channels.progress),
        grouping.clone(),
    )
    .await?;
    let candidates = clusters.candidates;
    let investigating = investigating_coverage(&grouping, batch_count, candidates.len())?;
    let context = Context {
        claim: channels.claim,
        candidates: candidates.clone(),
        examined: screened.examined,
        read: channels.read,
        model: channels.model,
        progress: Arc::clone(&channels.progress),
        coverage: investigating.clone(),
    };
    let mut investigations = investigate_candidates(context)?;
    let (findings, inconclusive) = collect_investigations(&mut investigations).await?;
    let investigated = i64::try_from(candidates.len())?;
    let coverage = Coverage {
        investigated,
        inconclusive,
        ..investigating.clone()
    };
    Ok(JobResult {
        findings,
        assessments: screened.assessments,
        coverage,
        error: String::new(),
    })
}

async fn collect_investigations(
    investigations: &mut Investigations,
) -> Result<(Vec<FindingDraft>, i64), CoordinatorError> {
    let mut findings = Vec::new();
    let mut inconclusive: i64 = 0;
    while let Some(item) = investigations.next().await {
        match item?.finding {
            Some(finding) => findings.push(finding),
            None => inconclusive = inconclusive.saturating_add(1),
        }
    }
    Ok((findings, inconclusive))
}

pub(super) fn limited(semaphore: Arc<Semaphore>, model: ModelCall) -> ModelCall {
    let gated_call = move |request: ModelRequest| -> ModelFuture {
        let permit_source = Arc::clone(&semaphore);
        let inner_model = Arc::clone(&model);
        Box::pin(async move {
            let _permit = permit_source
                .acquire()
                .await
                .map_err(|error| AnalysisError::Callback(error.to_string()))?;
            inner_model(request).await
        })
    };
    Arc::new(gated_call)
}

fn base_coverage(sample: &Sample) -> Result<Coverage, CoordinatorError> {
    let selected = i64::try_from(sample.executions.len())?;
    Ok(Coverage {
        eligible: sample.eligible,
        selected,
        screened: 0,
        investigated: 0,
        inconclusive: 0,
        grouping_batches: 0,
        grouped_batches: 0,
        candidates: 0,
        partial: 0,
        unassessable: 0,
    })
}

fn screened_coverage(base: &Coverage, examined: &[Examined]) -> Result<Coverage, CoordinatorError> {
    let screened = i64::try_from(examined.len())?;
    let partial = i64::try_from(examined.iter().filter(|item| item.partial).count())?;
    let unassessable = i64::try_from(examined.iter().filter(|item| item.cannot_assess).count())?;
    Ok(Coverage {
        screened,
        partial,
        unassessable,
        ..base.clone()
    })
}

fn grouping_coverage(coverage: &Coverage, batches: usize) -> Result<Coverage, CoordinatorError> {
    let grouping_batches = i64::try_from(batches)?;
    Ok(Coverage {
        grouping_batches,
        ..coverage.clone()
    })
}

fn investigating_coverage(
    grouping: &Coverage,
    batches: usize,
    candidate_count: usize,
) -> Result<Coverage, CoordinatorError> {
    let grouped_batches = i64::try_from(batches)?;
    let candidates = i64::try_from(candidate_count)?;
    Ok(Coverage {
        grouped_batches,
        candidates,
        ..grouping.clone()
    })
}

fn unique_checks(observations: &[Observation], kind: ObservationKind) -> Vec<String> {
    let mut checks: Vec<String> = observations
        .iter()
        .filter(|observation| observation.kind == kind)
        .map(|observation| observation.check_id.clone())
        .collect();
    checks.sort();
    checks.dedup();
    checks
}
