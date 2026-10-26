use super::{ANALYSIS_CONCURRENCY, AnalysisError, ReadContent, Record};
use crate::models::execution::{Execution, ExecutionContent, TracePart};
use crate::models::finding::Evidence;
use crate::models::messages::{ModelRequest, ModelResult};
use crate::trace_store::TraceStore;
use garde::Validate;
use std::convert::identity;
use std::future::Future;
use std::vec::IntoIter;
use tokio::task::JoinSet;

use crate::models::finding::FindingStatus;
use crate::models::messages::Claim;

pub async fn structured_response<T, F, Fut, V>(
    request: ModelRequest,
    model: F,
    validate: V,
) -> Result<T, AnalysisError>
where
    T: Record + Validate<Context = ()>,
    V: Fn(&T) -> Option<String>,
    F: Fn(ModelRequest) -> Fut,
    Fut: Future<Output = Result<ModelResult, AnalysisError>>,
{
    let first = model(request.clone()).await?;
    match parse_record::<T, V>(&first.content, &validate) {
        Ok(value) => Ok(value),
        Err(problem) => {
            let repair = ModelRequest {
                prompt: format!(
                    "{}\nYour previous response did not match the required response contract. Generate a new response from the original evidence, correcting these validation errors: {problem}",
                    request.prompt
                ),
                purpose: request.purpose,
            };
            parse_record::<T, V>(&model(repair).await?.content, &validate)
        }
    }
}

fn parse_record<T, V>(content: &str, validate: &V) -> Result<T, AnalysisError>
where
    T: Record + Validate<Context = ()>,
    V: Fn(&T) -> Option<String>,
{
    let value = T::model_validate_json(content.as_bytes())?;
    if let Some(problem) = validate(&value) {
        return Err(AnalysisError::Validation(problem));
    }
    Ok(value)
}

#[must_use]
pub fn evidence_valid(evidence: &Evidence, parts: &[TracePart]) -> bool {
    for part in parts {
        if part.execution_id == evidence.execution_id
            && part.span_id == evidence.span_id
            && part
                .content
                .split("\n[... content omitted ...]\n")
                .any(|segment| segment.contains(&evidence.quote))
        {
            return true;
        }
    }
    false
}

pub struct ConcurrentResults<Item, Output, Operation> {
    remaining: IntoIter<Item>,
    operation: Operation,
    concurrency: usize,
    tasks: JoinSet<Result<Output, AnalysisError>>,
}

pub fn concurrent_results<Item, Output, Operation, Work>(
    items: Vec<Item>,
    operation: Operation,
    concurrency: Option<usize>,
) -> Result<ConcurrentResults<Item, Output, Operation>, AnalysisError>
where
    Operation: Fn(Item) -> Work,
    Work: Future<Output = Result<Output, AnalysisError>> + Send + 'static,
    Output: Send + 'static,
{
    let limit = concurrency.unwrap_or(ANALYSIS_CONCURRENCY);
    if limit == 0 {
        return Err(AnalysisError::Validation(String::from(
            "concurrency must be positive",
        )));
    }
    Ok(ConcurrentResults {
        remaining: items.into_iter(),
        operation,
        concurrency: limit,
        tasks: JoinSet::new(),
    })
}

impl<Item, Output, Operation> ConcurrentResults<Item, Output, Operation>
where
    Output: Send + 'static,
{
    pub async fn next<Work>(&mut self) -> Option<Result<Output, AnalysisError>>
    where
        Operation: Fn(Item) -> Work,
        Work: Future<Output = Result<Output, AnalysisError>> + Send + 'static,
    {
        while self.tasks.len() < self.concurrency {
            match self.remaining.next() {
                Some(item) => {
                    self.tasks.spawn((self.operation)(item));
                }
                None => break,
            }
        }
        let completed = self.tasks.join_next().await?;
        let result = completed.map_err(AnalysisError::Join).and_then(identity);
        if result.is_err() {
            self.tasks.shutdown().await;
            self.remaining = Vec::new().into_iter();
        }
        Some(result)
    }
}

pub fn partition_items<T, S>(
    items: &[T],
    size: S,
    limit: usize,
) -> Result<Vec<Vec<T>>, AnalysisError>
where
    T: Clone,
    S: Fn(&T) -> usize,
{
    if limit == 0 {
        return Err(AnalysisError::Validation(String::from(
            "partition limit must be positive",
        )));
    }
    let mut batches: Vec<Vec<T>> = Vec::new();
    for item in items {
        let item_size = size(item);
        if item_size > limit {
            return Err(AnalysisError::Validation(String::from(
                "item exceeds partition limit",
            )));
        }
        let starts_batch = batches.last().is_none_or(|batch| {
            batch
                .iter()
                .map(&size)
                .try_fold(item_size, usize::checked_add)
                .is_none_or(|total| total > limit)
        });
        if starts_batch {
            batches.push(Vec::new());
        }
        if let Some(batch) = batches.last_mut() {
            batch.push(item.clone());
        }
    }
    Ok(batches)
}

pub fn partition_content(
    parts: &[TracePart],
    limit: usize,
) -> Result<Vec<Vec<TracePart>>, AnalysisError> {
    let sized = parts
        .iter()
        .map(|part| Ok((part, serde_json::to_vec(part)?.len().saturating_add(20))))
        .collect::<Result<Vec<_>, AnalysisError>>()?;
    Ok(partition_items(&sized, |(_, size)| *size, limit)?
        .into_iter()
        .map(|batch| batch.into_iter().map(|(part, _)| part.clone()).collect())
        .collect())
}

pub async fn read_execution(
    execution: Execution,
    read: &ReadContent,
    store: &mut TraceStore,
) -> Result<ExecutionContent, AnalysisError> {
    let mut cursor = String::new();
    let mut partial = false;
    loop {
        let mut page = read(execution.id.clone(), cursor.clone(), 0).await?;
        store.add(&page.parts)?;
        partial |= page.partial;
        match &page.next_cursor {
            Some(next) if !next.is_empty() && next != &cursor => cursor.clone_from(next),
            Some(_) | None => {
                page.parts.clear();
                page.partial = partial;
                return Ok(page);
            }
        }
    }
}

pub type FeedbackRow = (String, String, String, String, String);
pub fn feedback_pages(
    claim: &Claim,
    check_id: Option<&str>,
) -> Result<Vec<Vec<FeedbackRow>>, AnalysisError> {
    let entries: Vec<_> = claim
        .findings
        .iter()
        .filter(|finding| check_id.is_none_or(|id| finding.check_id == id))
        .map(|finding| {
            (
                finding.id.clone(),
                finding.check_id.clone(),
                finding.title.clone(),
                String::from(match finding.status {
                    FindingStatus::Open => "open",
                    FindingStatus::Resolved => "resolved",
                    FindingStatus::Dismissed => "dismissed",
                }),
                finding.reason.clone(),
            )
        })
        .collect();
    partition_items(
        &entries,
        |row| serde_json::to_vec(row).map_or(usize::MAX, |json| json.len().saturating_add(1)),
        7998,
    )
}
