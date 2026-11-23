#![expect(
    clippy::disallowed_types,
    reason = "tokio's task_local macro keeps its slot in a thread-local RefCell; that is the runtime's storage, not a borrow-checked value of ours"
)]

use std::error::Error;
use std::future::Future;

use async_trait::async_trait;
use betterlite_anthropic::native_compaction::protocol::{CompactionProtocol, Mapping};
use tokio::task::task_local;

mod tests;

// LiteLLM keeps the child flag and the parent pair in two context variables; being inside this scope is the child flag
task_local! {
    static NATIVE_COMPACTION_PARENT: Option<(String, String)>;
}

#[async_trait]
pub trait CompactionExecutor: Send + Sync {
    async fn execute(
        &self,
        protocol: CompactionProtocol,
        payload: &Mapping,
        parent_model: Option<&str>,
        logging_disabled: bool,
    ) -> Result<Mapping, CompactionExecutorError>;
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CompactionExecutorError(#[source] pub Box<dyn Error + Send + Sync>);

pub async fn native_compaction_call<F>(
    parent_model: Option<&str>,
    compactor: Option<&str>,
    call: F,
) -> F::Output
where
    F: Future,
{
    let pair = parent_model
        .zip(compactor)
        .filter(|(parent, compactor)| !parent.is_empty() && !compactor.is_empty())
        .map(|(parent, compactor)| (parent.to_owned(), compactor.to_owned()));
    NATIVE_COMPACTION_PARENT.scope(pair, call).await
}

#[must_use]
pub fn native_compaction_parent(model: &str) -> Option<String> {
    NATIVE_COMPACTION_PARENT
        .try_with(|pair| match pair {
            Some((parent, compactor)) if compactor == model => Some(parent.clone()),
            Some(_) | None => None,
        })
        .unwrap_or_default()
}

#[must_use]
pub fn is_native_compaction_call() -> bool {
    NATIVE_COMPACTION_PARENT.try_with(|_| ()).is_ok()
}
