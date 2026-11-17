use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_router::semantic_router::service::{CallerContext, RouteEmbedder};
use betterlite_server::routing::semantic_router::proxy_server_request;
use indexmap::IndexMap;

use super::{DocumentCall, QueryCall};

pub(super) struct Recorder {
    vectors: IndexMap<String, Vec<f64>>,
    pub(super) documents: Mutex<Vec<DocumentCall>>,
    pub(super) queries: Mutex<Vec<QueryCall>>,
    document_failures: AtomicUsize,
    query_failures: AtomicUsize,
}

impl Recorder {
    pub(super) fn new(
        vectors: IndexMap<String, Vec<f64>>,
        document_failures: usize,
        query_failures: usize,
    ) -> Self {
        Self {
            vectors,
            documents: Mutex::new(Vec::new()),
            queries: Mutex::new(Vec::new()),
            document_failures: AtomicUsize::new(document_failures),
            query_failures: AtomicUsize::new(query_failures),
        }
    }

    fn document_vectors(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f64>>, io::Error> {
        begin(
            &self.documents,
            DocumentCall {
                model: model.to_owned(),
                input: texts.to_vec(),
            },
            &self.document_failures,
            "the probe failed the document embedding",
        )?;
        lookup_vectors(&self.vectors, texts)
    }
}

fn lookup_vectors(
    vectors: &IndexMap<String, Vec<f64>>,
    texts: &[String],
) -> Result<Vec<Vec<f64>>, io::Error> {
    let mut found = Vec::with_capacity(texts.len());
    for text in texts {
        let Some(vector) = vectors.get(text) else {
            return Err(io::Error::other(format!("no probe vector for {text:?}")));
        };
        found.push(vector.clone());
    }
    Ok(found)
}

fn take_failure(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
}

fn begin<T>(
    calls: &Mutex<Vec<T>>,
    call: T,
    failures: &AtomicUsize,
    message: &str,
) -> Result<(), io::Error> {
    calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(call);
    if take_failure(failures) {
        return Err(io::Error::other(message));
    }
    Ok(())
}

impl RouteEmbedder for Recorder {
    type Error = io::Error;

    fn embed_documents(
        &self,
        model: &str,
        texts: Vec<String>,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        async move { self.document_vectors(model, &texts) }
    }

    fn embed_query(
        &self,
        model: &str,
        text: String,
        raw_text: &str,
        caller: &CallerContext,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        let call = QueryCall {
            model: model.to_owned(),
            input: vec![text.clone()],
            metadata: caller.metadata.clone(),
            litellm_metadata: caller.litellm_metadata.clone(),
            proxy_server_request: proxy_server_request(model, raw_text),
            turn_off_message_logging: caller.turn_off_message_logging,
            litellm_session_id: caller.litellm_session_id.clone(),
            litellm_trace_id: caller.litellm_trace_id.clone(),
        };
        async move {
            begin(
                &self.queries,
                call,
                &self.query_failures,
                "the probe failed the query embedding",
            )?;
            lookup_vectors(&self.vectors, std::slice::from_ref(&text))
        }
    }
}

pub(super) fn call_counts(recorder: &Recorder) -> (usize, usize) {
    let documents = recorder
        .documents
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let queries = recorder
        .queries
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    (documents.len(), queries.len())
}

pub(super) fn delta<T: Clone>(calls: &Mutex<Vec<T>>, start: usize) -> Vec<T> {
    calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .skip(start)
        .cloned()
        .collect()
}
