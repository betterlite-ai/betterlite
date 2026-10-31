use super::investigate;
use crate::analysis::primitives::{ConcurrentResults, concurrent_results};
use crate::analysis::{
    AnalysisError, Candidate, Examined, Investigation, ModelCall, ReadContent, ReportProgress,
};
use crate::models::execution::Coverage;
use crate::models::messages::Claim;
use std::future::Future;
use std::pin::Pin;
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

type BoxedWork = Pin<Box<dyn Future<Output = Result<Investigation, AnalysisError>> + Send>>;
type BoxedOperation = Box<dyn Fn(Candidate) -> BoxedWork + Send>;

pub struct Investigations {
    results: ConcurrentResults<Candidate, Investigation, BoxedOperation>,
    coverage: Coverage,
    progress: ReportProgress,
    completed: i64,
    inconclusive: i64,
}

pub fn investigate_candidates(context: Context) -> Result<Investigations, AnalysisError> {
    let concurrency = usize::try_from(context.claim.job.settings.concurrency).map_err(|error| {
        AnalysisError::Validation(format!("concurrency is out of range: {error}"))
    })?;
    let claim = context.claim;
    let examined = context.examined;
    let read = context.read;
    let model = context.model;
    let operation: BoxedOperation = Box::new(move |candidate: Candidate| -> BoxedWork {
        Box::pin(investigate(
            claim.clone(),
            candidate,
            examined.clone(),
            Arc::clone(&read),
            Arc::clone(&model),
        ))
    });
    let results = concurrent_results(context.candidates, operation, Some(concurrency))?;
    Ok(Investigations {
        results,
        coverage: context.coverage,
        progress: context.progress,
        completed: 0,
        inconclusive: 0,
    })
}

impl Investigations {
    pub async fn next(&mut self) -> Option<Result<Investigation, AnalysisError>> {
        let investigation = match self.results.next().await? {
            Ok(value) => value,
            Err(error) => return Some(Err(error)),
        };
        self.completed = self.completed.saturating_add(1);
        self.inconclusive = self
            .inconclusive
            .saturating_add(i64::from(investigation.finding.is_none()));
        let mut current = self.coverage.clone();
        current.investigated = self.completed;
        current.inconclusive = self.inconclusive;
        if let Err(error) =
            (self.progress)(String::from("Checking original evidence"), current).await
        {
            return Some(Err(error));
        }
        Some(Ok(investigation))
    }
}
