use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_router::semantic_embedding_inputs::ConfiguredTokenLimits;
use betterlite_router::semantic_embeddings::{
    EmbeddedReply, EmbeddingCalls, EmbeddingFailure, EmbeddingRequest,
};

use super::input::{ModelRow, Outcome};

pub(crate) struct HelperRouter<'rows> {
    pub(crate) rows: &'rows [ModelRow],
}

impl ConfiguredTokenLimits for HelperRouter<'_> {
    fn get_configured_token_limits(&self, model_name: &str) -> (Option<i64>, Option<i64>) {
        let found = self.rows.iter().find(|row| row.model_name == model_name);
        (found.and_then(|row| row.max_input_tokens), None)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HelperResponse {
    pub(crate) vectors: Vec<Vec<f64>>,
    pub(crate) model: String,
}

impl EmbeddedReply for HelperResponse {
    fn first_embedding(&self) -> Option<Vec<f64>> {
        self.vectors.first().cloned()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted embedding call failed")]
pub(crate) struct HelperCallFailure {
    pub(crate) status: u16,
}

impl EmbeddingFailure for HelperCallFailure {
    fn status_code(&self) -> Option<u16> {
        Some(self.status)
    }
}

#[derive(Clone, Default)]
pub(crate) struct CallLog {
    requests: Arc<Mutex<Vec<EmbeddingRequest>>>,
}

impl CallLog {
    pub(crate) fn requests(&self) -> Vec<EmbeddingRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }
}

pub(crate) struct RecordingCalls {
    pub(crate) log: CallLog,
    pub(crate) vectors: Vec<Vec<f64>>,
    pub(crate) model: String,
    pub(crate) outcome: Outcome,
}

impl EmbeddingCalls for RecordingCalls {
    type Response = HelperResponse;
    type Error = HelperCallFailure;

    fn embedding(
        &self,
        request: &EmbeddingRequest,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send {
        self.record(request);
        ready(self.finish())
    }

    fn aembedding(
        &self,
        request: &EmbeddingRequest,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send {
        self.record(request);
        ready(self.finish())
    }
}

impl RecordingCalls {
    fn record(&self, request: &EmbeddingRequest) {
        if let Ok(mut requests) = self.log.requests.lock() {
            requests.push(request.clone());
        }
    }

    fn finish(&self) -> Result<HelperResponse, HelperCallFailure> {
        match self.outcome {
            Outcome::Success => Ok(HelperResponse {
                vectors: self.vectors.clone(),
                model: self.model.clone(),
            }),
            Outcome::Failure => Err(HelperCallFailure { status: 500 }),
        }
    }
}
