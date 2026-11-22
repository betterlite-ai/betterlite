use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_router::semantic_router::service::{CallerContext, RouteEmbedder};

use super::Input;
use super::wire::{CallOut, count, vector};
use crate::router_probes::error::OperationError;

#[derive(Debug, thiserror::Error)]
pub(super) enum EmbedError {
    #[error("the probe has no vector for {text:?}")]
    Missing { text: String },
    #[error("the probe was told to fail the {stage} call")]
    Injected { stage: &'static str },
}

pub(super) struct TableEmbedder {
    vectors: BTreeMap<String, Vec<f64>>,
    calls: Mutex<Vec<CallOut>>,
    document_failures: AtomicU32,
    query_failures: AtomicU32,
}

impl TableEmbedder {
    pub(super) fn new(input: &Input) -> Result<Self, OperationError> {
        let mut vectors = BTreeMap::new();
        for (text, value) in &input.embeddings {
            vectors.insert(text.clone(), vector(value)?);
        }
        Ok(Self {
            vectors,
            calls: Mutex::new(Vec::new()),
            document_failures: AtomicU32::new(count(input.document_failures.as_ref(), 0)?),
            query_failures: AtomicU32::new(count(input.query_failures.as_ref(), 0)?),
        })
    }

    pub(super) fn calls(&self) -> Vec<CallOut> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, stage: &str, model: &str, inputs: &[String]) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CallOut {
                stage: stage.to_owned(),
                model: model.to_owned(),
                inputs: inputs.to_vec(),
            });
    }

    fn collect(
        &self,
        stage: &'static str,
        model: &str,
        texts: Vec<String>,
        failures: &AtomicU32,
    ) -> Result<Vec<Vec<f64>>, EmbedError> {
        self.record(stage, model, &texts);
        if take_failure(failures) {
            return Err(EmbedError::Injected { stage });
        }
        answer(&self.vectors, texts)
    }
}

fn answer(
    vectors: &BTreeMap<String, Vec<f64>>,
    texts: Vec<String>,
) -> Result<Vec<Vec<f64>>, EmbedError> {
    let mut found = Vec::with_capacity(texts.len());
    for text in texts {
        let vector = vectors
            .get(&text)
            .cloned()
            .ok_or(EmbedError::Missing { text })?;
        found.push(vector);
    }
    Ok(found)
}

impl RouteEmbedder for TableEmbedder {
    type Error = EmbedError;

    fn embed_documents(
        &self,
        model: &str,
        texts: Vec<String>,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        ready(self.collect("documents", model, texts, &self.document_failures))
    }

    fn embed_query(
        &self,
        model: &str,
        text: String,
        _raw_text: &str,
        _caller: &CallerContext,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        ready(self.collect("query", model, vec![text], &self.query_failures))
    }
}

fn take_failure(counter: &AtomicU32) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
}
