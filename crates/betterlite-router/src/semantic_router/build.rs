use std::error::Error;
use std::sync::Arc;

use tokio::sync::watch;

use super::error::{EmbeddingStage, RouterError};
use super::index::RouteIndex;
use super::select::BuiltEntry;
use super::service::RouteEmbedder;
use super::text::clamp;
use crate::semantic_math::validate_documents;

// the encoder probes the embedding width with this text before it embeds any utterance
pub(super) const DIMENSION_PROBE_TEXT: &str = "test";

pub(super) enum BuildState<E>
where
    E: Error + Send + Sync + 'static,
{
    Building,
    Done(Result<Arc<Vec<BuiltEntry>>, Arc<RouterError<E>>>),
}

impl<E> Clone for BuildState<E>
where
    E: Error + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        match self {
            Self::Building => Self::Building,
            Self::Done(outcome) => Self::Done(outcome.clone()),
        }
    }
}

pub(super) struct BuildSlot<E>
where
    E: Error + Send + Sync + 'static,
{
    pub(super) built: Option<Arc<Vec<BuiltEntry>>>,
    pub(super) waiters: Option<watch::Receiver<BuildState<E>>>,
}

pub(super) async fn wait_for_build<E>(
    mut receiver: watch::Receiver<BuildState<E>>,
) -> Result<Arc<Vec<BuiltEntry>>, Arc<RouterError<E>>>
where
    E: Error + Send + Sync + 'static,
{
    loop {
        match receiver.borrow_and_update().clone() {
            BuildState::Done(outcome) => return outcome,
            BuildState::Building => {}
        }
        if receiver.changed().await.is_err() {
            return Err(Arc::new(RouterError::BuildAborted));
        }
    }
}

pub(super) async fn build_entries<E>(
    index: &RouteIndex,
    embedder: &E,
    model: &str,
    max_input_chars: i64,
) -> Result<Arc<Vec<BuiltEntry>>, RouterError<E::Error>>
where
    E: RouteEmbedder,
{
    let probe = embedder
        .embed_documents(
            model,
            vec![clamp(DIMENSION_PROBE_TEXT, max_input_chars).to_owned()],
        )
        .await
        .map_err(|source| RouterError::Embedding {
            stage: EmbeddingStage::Documents,
            source,
        })?;
    if probe.len() != 1 {
        return Err(RouterError::VectorCount {
            stage: EmbeddingStage::Documents,
            expected: 1,
            found: probe.len(),
        });
    }
    let mut texts = Vec::with_capacity(index.rows.len());
    for row in &index.rows {
        texts.push(clamp(&row.text, max_input_chars).to_owned());
    }
    let vectors = if texts.is_empty() {
        Vec::new()
    } else {
        let expected = texts.len();
        let vectors = embedder
            .embed_documents(model, texts)
            .await
            .map_err(|source| RouterError::Embedding {
                stage: EmbeddingStage::Documents,
                source,
            })?;
        if vectors.len() != expected {
            return Err(RouterError::VectorCount {
                stage: EmbeddingStage::Documents,
                expected,
                found: vectors.len(),
            });
        }
        validate_documents(&vectors).map_err(|source| RouterError::Vectors {
            stage: EmbeddingStage::Documents,
            source,
        })?;
        vectors
    };
    let mut entries = Vec::with_capacity(index.rows.len());
    for (row, vector) in index.rows.iter().zip(vectors) {
        entries.push(BuiltEntry {
            route: row.route.clone(),
            threshold: row.threshold,
            vector,
        });
    }
    Ok(Arc::new(entries))
}
