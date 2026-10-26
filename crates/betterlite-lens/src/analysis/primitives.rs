use tokio::{sync::Semaphore, task::JoinSet};

use crate::models::messages::Claim;

pub async fn structured_response<T, F, Fut>(
    request: ModelRequest,
    model: F,
) -> Result<T, AnalysisError>
where
    T: Record + Validate<Context = ()>,
    F: Fn(ModelRequest) -> Fut,
    Fut: Future<Output = Result<ModelResult, AnalysisError>>,
{
    let first = model(request.clone()).await?;
    match parse_record::<T>(&first.content) {
        Ok(value) => Ok(value),
        Err(problem) => {
            let repair = ModelRequest {
                prompt: format!(
                    "{}\nYour previous response was invalid: {problem}",
                    request.prompt
                ),
                purpose: request.purpose,
            };
            parse_record::<T>(&model(repair).await?.content)
        }
    }
}

fn parse_record<T>(content: &str) -> Result<T, AnalysisError>
where
    T: Record + Validate<Context = ()>,
{
    let value: T = serde_json::from_str(content)?;
    value
        .validate()
        .map_err(|error| AnalysisError::Validation(error.to_string()))?;
    Ok(value)
}

#[must_use]
pub fn evidence_valid(evidence: &Evidence, parts: &[TracePart]) -> bool {
    let valid = parts.iter().any(|part| {
        part.execution_id == evidence.execution_id
            && part.span_id == evidence.span_id
            && part
                .content
                .split("\n[... content omitted ...]\n")
                .any(|segment| segment.contains(&evidence.quote))
    });
    valid
}

pub async fn concurrent_results<Item, ResultValue, Operation, FutureValue>(
    items: Vec<Item>,
    operation: Operation,
    concurrency: usize,
) -> Result<Vec<ResultValue>, AnalysisError>
where
    Item: Send + 'static,
    ResultValue: Send + 'static,
    Operation: Fn(Item) -> FutureValue + Send + Sync + Copy + 'static,
    FutureValue: Future<Output = Result<ResultValue, AnalysisError>> + Send + 'static,
{
    if concurrency == 0 {
        return Err(AnalysisError::Validation(String::from(
            "concurrency must be positive",
        )));
    }
    let result_capacity = items.len();
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = JoinSet::new();
    for item in items {
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .map_err(|error| AnalysisError::Callback(error.to_string()))?;
        tasks.spawn(async move {
            let result = operation(item).await;
            drop(permit);
            result
        });
    }
    let mut results = Vec::with_capacity(result_capacity);
    while let Some(task) = tasks.join_next().await {
        results.push(task.map_err(|error| AnalysisError::Callback(error.to_string()))??);
    }
    Ok(results)
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
    let batches = partition_items(
        parts,
        |part| serde_json::to_vec(part).map_or(usize::MAX, |json| json.len().saturating_add(20)),
        limit,
    )?;
    Ok(batches)
}

pub async fn read_execution(
    execution: Execution,
    read: &ReadContent,
    store: &mut TraceStore,
) -> Result<ExecutionContent, AnalysisError> {
    let mut cursor = String::new();
    let mut partial = false;
    loop {
        let page = read(execution.id.clone(), cursor.clone(), 0).await?;
        store.add(&page.parts)?;
        partial |= page.partial;
        match page.next_cursor {
            None => {
                return Ok(ExecutionContent {
                    execution,
                    parts: Vec::new(),
                    next_cursor: None,
                    partial,
                });
            }
            Some(next) if next == cursor => {
                return Ok(ExecutionContent {
                    execution,
                    parts: Vec::new(),
                    next_cursor: None,
                    partial: true,
                });
            }
            Some(next) => cursor = next,
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
                format!("{:?}", finding.status),
                finding.reason.clone(),
            )
        })
        .collect();
    partition_items(
        &entries,
        |row| serde_json::to_vec(row).map_or(usize::MAX, |json| json.len()),
        8000,
    )
}
