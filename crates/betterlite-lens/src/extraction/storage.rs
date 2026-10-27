use crate::analysis::AnalysisError;
use crate::models::execution::TracePart;
use crate::trace_store::{CatalogRow, TraceStore, TraceStoreError, trace_store};
use tokio::task::JoinSet;

pub(super) struct Stored {
    pub inner: Option<TraceStore>,
}

pub(super) async fn open() -> Result<Stored, AnalysisError> {
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(trace_store);
    let store = tasks
        .join_next()
        .await
        .ok_or(TraceStoreError::Unavailable)???;
    Ok(Stored { inner: Some(store) })
}

impl Stored {
    pub(super) async fn access<T, F>(&mut self, operation: F) -> Result<T, AnalysisError>
    where
        F: FnOnce(&TraceStore) -> Result<T, AnalysisError> + Send + 'static,
        T: Send + 'static,
    {
        let store = self.inner.take().ok_or(TraceStoreError::Unavailable)?;
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            let result = operation(&store);
            (store, result)
        });
        let (stored, result) = tasks
            .join_next()
            .await
            .ok_or(TraceStoreError::Unavailable)??;
        self.inner = Some(stored);
        result
    }
}

pub(super) struct Summary {
    pub root: Option<TracePart>,
    pub root_count: usize,
    pub span_count: usize,
    pub catalogs: Vec<Vec<CatalogRow>>,
}

pub(super) fn summarize(store: &TraceStore) -> Result<Summary, AnalysisError> {
    let mut root = None;
    let mut root_count = 0_usize;
    for result in store.parts() {
        let part = result?;
        if part.parent_span_id.is_empty() {
            root_count = root_count.saturating_add(1);
            if root.is_none() {
                root = Some(part);
            }
        }
    }
    Ok(Summary {
        root,
        root_count,
        span_count: store.count()?,
        catalogs: store.catalogs(root_count).collect::<Result<_, _>>()?,
    })
}
