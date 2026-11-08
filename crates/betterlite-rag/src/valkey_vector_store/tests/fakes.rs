use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_provider::api::vector_store_contracts::config::Params;
use betterlite_provider::api::vector_store_contracts::error::VectorStoreEmbeddingError;
use secrecy::{ExposeSecret as _, SecretString};

use crate::valkey_vector_store::error::ValkeyError;
use crate::valkey_vector_store::ports::{
    SocketTimeouts, ValkeyConnector, ValkeyDocument, ValkeyEmbedder, ValkeySearchParams,
    ValkeySearchPort,
};
use crate::valkey_vector_store::query::ValkeyKnnQuery;

pub(crate) fn document(id: &str, fields: &[(&str, &str)]) -> ValkeyDocument {
    ValkeyDocument {
        id: id.to_owned(),
        fields: fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

pub(crate) fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("a params fixture")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EmbedCall {
    pub(crate) model: String,
    pub(crate) text: String,
    pub(crate) configuration: Params,
}

#[derive(Default)]
pub(crate) struct FakeEmbedder {
    pub(crate) calls: Mutex<Vec<EmbedCall>>,
    pub(crate) vectors: Vec<Vec<f32>>,
    pub(crate) fail: bool,
}

impl FakeEmbedder {
    pub(crate) fn answering(vectors: Vec<Vec<f32>>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            vectors,
            fail: false,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            vectors: Vec::new(),
            fail: true,
        }
    }

    pub(crate) fn calls(&self) -> Vec<EmbedCall> {
        self.calls.lock().expect("the embedder log").clone()
    }
}

#[async_trait]
impl ValkeyEmbedder for FakeEmbedder {
    async fn embed(
        &self,
        model: &str,
        text: &str,
        configuration: &Params,
    ) -> Result<Vec<Vec<f32>>, VectorStoreEmbeddingError> {
        self.calls
            .lock()
            .expect("the embedder log")
            .push(EmbedCall {
                model: model.to_owned(),
                text: text.to_owned(),
                configuration: configuration.clone(),
            });
        if self.fail {
            return Err(VectorStoreEmbeddingError::FieldType {
                field: "the_test_embedder",
                expected: "a success",
            });
        }
        Ok(self.vectors.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortCall {
    pub(crate) index: String,
    pub(crate) query: ValkeyKnnQuery,
    pub(crate) params: ValkeySearchParams,
}

#[derive(Clone, Default)]
pub(crate) struct PortLog {
    pub(crate) calls: Arc<Mutex<Vec<PortCall>>>,
    pub(crate) closed: Arc<AtomicBool>,
    pub(crate) searches: Arc<Mutex<usize>>,
}

impl PortLog {
    pub(crate) fn calls(&self) -> Vec<PortCall> {
        self.calls.lock().expect("the port log").clone()
    }

    pub(crate) fn closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub(crate) enum PortOutcome {
    Documents(Vec<ValkeyDocument>),
    Failure,
}

pub(crate) struct FakePort {
    pub(crate) log: PortLog,
    pub(crate) outcome: PortOutcome,
}

impl Drop for FakePort {
    fn drop(&mut self) {
        self.log.closed.store(true, Ordering::Relaxed);
    }
}

#[async_trait]
impl ValkeySearchPort for FakePort {
    async fn search(
        &self,
        index: &str,
        query: &ValkeyKnnQuery,
        params: &ValkeySearchParams,
    ) -> Result<Vec<ValkeyDocument>, ValkeyError> {
        self.log.calls.lock().expect("the port log").push(PortCall {
            index: index.to_owned(),
            query: query.clone(),
            params: params.clone(),
        });
        let mut searches = self.log.searches.lock().expect("the port count");
        *searches = searches.saturating_add(1);
        drop(searches);
        match &self.outcome {
            PortOutcome::Documents(documents) => Ok(documents.clone()),
            PortOutcome::Failure => Err(ValkeyError::UnexpectedReply {
                detail: "the test port failed",
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConnectCall {
    pub(crate) url: String,
    pub(crate) timeouts: SocketTimeouts,
}

pub(crate) struct FakeConnector {
    pub(crate) calls: Mutex<Vec<ConnectCall>>,
    pub(crate) log: PortLog,
    pub(crate) outcome: PortOutcome,
    pub(crate) fail: bool,
}

impl FakeConnector {
    pub(crate) fn new(outcome: PortOutcome) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            log: PortLog::default(),
            outcome,
            fail: false,
        }
    }

    pub(crate) fn calls(&self) -> Vec<ConnectCall> {
        self.calls.lock().expect("the connector log").clone()
    }
}

#[async_trait]
impl ValkeyConnector for FakeConnector {
    async fn connect(
        &self,
        url: &SecretString,
        timeouts: SocketTimeouts,
    ) -> Result<Box<dyn ValkeySearchPort>, ValkeyError> {
        self.calls
            .lock()
            .expect("the connector log")
            .push(ConnectCall {
                url: url.expose_secret().to_owned(),
                timeouts,
            });
        if self.fail {
            return Err(ValkeyError::UnexpectedReply {
                detail: "the test connector failed",
            });
        }
        Ok(Box::new(FakePort {
            log: self.log.clone(),
            outcome: self.outcome.clone(),
        }))
    }
}

pub(crate) fn text_documents() -> Vec<ValkeyDocument> {
    vec![
        document("doc-b", &[("text", "second"), ("vector_distance", "0.25")]),
        document("doc-a", &[("text", "first"), ("vector_distance", "0.5")]),
    ]
}

pub(crate) fn empty_params() -> Params {
    BTreeMap::new()
}
