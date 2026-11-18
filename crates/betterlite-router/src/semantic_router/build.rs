use std::error::Error;
use std::sync::Arc;

use tokio::sync::watch;
use tokio::time::timeout;

use super::error::{EmbeddingStage, RouterError};
use super::index::RouteIndex;
use super::select::BuiltEntry;
use super::service::{EMBEDDING_TIMEOUT, RouteEmbedder};
use super::text::clamp;
use crate::semantic_math::validate_documents;

// the encoder probes the embedding width with this text before it embeds any utterance
pub(super) const DIMENSION_PROBE_TEXT: &str = "test";

pub(super) enum BuildState<E>
where
    E: Error + Send + Sync + 'static,
{
    Building,
    Done(Result<Arc<[BuiltEntry]>, Arc<RouterError<E>>>),
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
    pub(super) built: Option<Arc<[BuiltEntry]>>,
    pub(super) waiters: Option<watch::Receiver<BuildState<E>>>,
}

pub(super) async fn wait_for_build<E>(
    mut receiver: watch::Receiver<BuildState<E>>,
) -> Result<Arc<[BuiltEntry]>, Arc<RouterError<E>>>
where
    E: Error + Send + Sync + 'static,
{
    loop {
        let state = receiver.borrow_and_update().clone();
        match state {
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
) -> Result<Arc<[BuiltEntry]>, RouterError<E::Error>>
where
    E: RouteEmbedder,
{
    let probe = clamp(DIMENSION_PROBE_TEXT, max_input_chars).to_owned();
    let _probe = embed_batch(embedder, model, vec![probe]).await?;
    let texts = index
        .rows
        .iter()
        .map(|row| clamp(&row.text, max_input_chars).to_owned())
        .collect::<Vec<String>>();
    if texts.is_empty() {
        return Ok(Arc::<[BuiltEntry]>::from(Vec::new()));
    }
    let vectors = embed_batch(embedder, model, texts).await?;
    validate_documents(&vectors).map_err(|source| RouterError::Vectors {
        stage: EmbeddingStage::Documents,
        source,
    })?;
    Ok(Arc::from(zip_entries(index, vectors)))
}

// one embedding call per batch, refused when the provider answers with a different number of vectors
async fn embed_batch<E>(
    embedder: &E,
    model: &str,
    texts: Vec<String>,
) -> Result<Vec<Vec<f64>>, RouterError<E::Error>>
where
    E: RouteEmbedder,
{
    let expected = texts.len();
    let answer = timeout(EMBEDDING_TIMEOUT, embedder.embed_documents(model, texts)).await;
    let vectors = match answer {
        Ok(Ok(vectors)) => vectors,
        Ok(Err(source)) => {
            return Err(RouterError::Embedding {
                stage: EmbeddingStage::Documents,
                source,
            });
        }
        Err(_elapsed) => {
            return Err(RouterError::Timeout {
                stage: EmbeddingStage::Documents,
                seconds: EMBEDDING_TIMEOUT.as_secs(),
            });
        }
    };
    if vectors.len() != expected {
        return Err(RouterError::VectorCount {
            stage: EmbeddingStage::Documents,
            expected,
            found: vectors.len(),
        });
    }
    Ok(vectors)
}

fn zip_entries(index: &RouteIndex, vectors: Vec<Vec<f64>>) -> Vec<BuiltEntry> {
    let mut entries = Vec::with_capacity(index.rows.len());
    for (row, vector) in index.rows.iter().zip(vectors) {
        entries.push(BuiltEntry {
            route: row.route.clone(),
            threshold: row.threshold,
            vector,
        });
    }
    entries
}
