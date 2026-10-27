use super::{
    prompt::{Context, review},
    storage::Stored,
};
use crate::analysis::primitives::concurrent_results;
use crate::analysis::{AnalysisError, ModelCall, ReadContent, SpanRead, TraceReview};
use crate::models::execution::TracePart;
use crate::trace_store::CatalogRow;
use std::{collections::HashSet, sync::Arc};

#[derive(Default)]
pub(super) struct Navigation {
    pub feedback_page: usize,
    pub must_decide: bool,
    pub previous: TraceReview,
    pub reads: Vec<SpanRead>,
    pub additional: Vec<TracePart>,
    pub seen: HashSet<usize>,
}

pub(super) async fn examine(
    context: &Context<'_>,
    catalog: &[CatalogRow],
    store: &mut Stored,
    read: &ReadContent,
    model: &ModelCall,
) -> Result<TraceReview, AnalysisError> {
    let mut state = Navigation::default();
    state.seen.insert(0);
    loop {
        let response = review(context, catalog, &state, model).await?;
        let next = response
            .feedback_page
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(state.feedback_page);
        if state.must_decide || (response.reads.is_empty() && next == state.feedback_page) {
            return Ok(response);
        }
        if next == state.feedback_page {
            advance(
                &mut state,
                &response.reads,
                context.execution.id.clone(),
                store,
                read,
            )
            .await?;
        } else {
            state.must_decide = next >= context.feedback.len() || !state.seen.insert(next);
            if !state.must_decide {
                state.feedback_page = next;
            }
        }
        state.previous = response;
    }
}

async fn advance(
    state: &mut Navigation,
    requests: &[SpanRead],
    execution: String,
    store: &mut Stored,
    read: &ReadContent,
) -> Result<(), AnalysisError> {
    let requested = select(requests, &state.reads, store).await?;
    let fetched = fetch(&requested, execution, read).await?;
    state.must_decide = !fetched
        .iter()
        .any(|part| !part.content.is_empty() && !state.additional.contains(part));
    if !state.must_decide {
        let stored_parts = fetched.clone();
        store
            .access(move |stored| {
                stored.add_reads(&stored_parts)?;
                Ok(())
            })
            .await?;
        state
            .reads
            .extend(requested.into_iter().map(|(request, _)| request));
        state.additional = fetched;
    }
    Ok(())
}

async fn fetch(
    requests: &[(SpanRead, String)],
    execution: String,
    read: &ReadContent,
) -> Result<Vec<TracePart>, AnalysisError> {
    let reader = Arc::clone(read);
    let mut results = concurrent_results(
        requests.to_vec(),
        move |(request, previous): (SpanRead, String)| {
            let future = reader(execution.clone(), previous, request.offset);
            async move {
                let content = future.await?;
                Ok(content
                    .parts
                    .into_iter()
                    .filter(|part| part.span_id == request.span_id)
                    .collect::<Vec<_>>())
            }
        },
        Some(2),
    )?;
    let mut fetched = Vec::new();
    while let Some(result) = results.next().await {
        fetched.extend(result?);
    }
    Ok(fetched)
}

async fn select(
    requests: &[SpanRead],
    completed: &[SpanRead],
    store: &mut Stored,
) -> Result<Vec<(SpanRead, String)>, AnalysisError> {
    let pending: Vec<_> = requests
        .iter()
        .filter(|request| {
            !completed
                .iter()
                .any(|done| done.span_id == request.span_id && done.offset == request.offset)
        })
        .cloned()
        .collect();
    store
        .access(move |stored| {
            let mut selected = Vec::new();
            for request in pending {
                if selected.len() < 2 && stored.get(&request.span_id)?.is_some() {
                    let previous = stored.previous(&request.span_id)?;
                    selected.push((request, previous));
                }
            }
            Ok(selected)
        })
        .await
}
