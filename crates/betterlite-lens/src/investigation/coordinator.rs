use super::investigate;
use crate::analysis::primitives::concurrent_results;
use crate::analysis::{
    AnalysisError, Candidate, Examined, Investigation, ModelCall, ReadContent, ReportProgress,
};
use crate::models::execution::Coverage;
use crate::models::messages::Claim;
use std::sync::Arc;

pub struct Context {
    pub claim: Claim,
    pub candidates: Vec<Candidate>,
    pub examined: Vec<Examined>,
    pub read: ReadContent,
    pub model: ModelCall,
    pub progress: ReportProgress,
    pub coverage: Coverage,
}

pub async fn investigate_candidates(context: Context) -> Result<Vec<Investigation>, AnalysisError> {
    let concurrency = usize::try_from(context.claim.job.settings.concurrency).map_err(|error| {
        AnalysisError::Validation(format!("concurrency is out of range: {error}"))
    })?;
    let mut results = concurrent_results(
        context.candidates,
        move |candidate| {
            investigate(
                context.claim.clone(),
                candidate,
                context.examined.clone(),
                Arc::clone(&context.read),
                Arc::clone(&context.model),
            )
        },
        Some(concurrency),
    )?;
    let mut completed = 0_i64;
    let mut inconclusive = 0_i64;
    let mut output = Vec::new();
    while let Some(result) = results.next().await {
        let investigation = result?;
        completed = completed.saturating_add(1);
        inconclusive = inconclusive.saturating_add(i64::from(investigation.finding.is_none()));
        let mut current = context.coverage.clone();
        current.investigated = completed;
        current.inconclusive = inconclusive;
        (context.progress)(String::from("Checking original evidence"), current).await?;
        output.push(investigation);
    }
    Ok(output)
}
