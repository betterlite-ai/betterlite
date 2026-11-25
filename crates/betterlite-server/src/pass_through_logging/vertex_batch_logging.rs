use std::sync::Arc;

use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::batches::records::BatchObject;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::pass_through_logging::anthropic_batch::{
    BatchLoggingResponse, ManagedBatchRegistry, ResponseIds, RouterModelIds,
};

mod error;
mod job;
mod record;
mod registration;
mod tests;

const VERTEX_BATCH_PREDICTION_JOBS_ROUTE: &str = "batchPredictionJobs";
const UNKNOWN_MODEL: &str = "unknown";
const SUCCESS_STATUS: u16 = 200;

pub struct VertexBatchLogging {
    router: Option<Arc<dyn RouterModelIds>>,
    registry: Option<Arc<dyn ManagedBatchRegistry>>,
    ids: Arc<dyn ResponseIds>,
}

#[derive(Clone, Copy)]
pub struct VertexBatchInput<'input> {
    pub status_code: u16,
    pub body: &'input [u8],
    pub url_route: &'input str,
    pub start_time: DateTime<Utc>,
    pub request_metadata: &'input JsonObject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VertexBatchLogged {
    pub result: BatchLoggingResponse,
    pub kwargs: VertexBatchKwargs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VertexBatchKwargs {
    pub response_cost: Decimal,
    pub model: String,
    pub batch_id: Option<String>,
    pub unified_object_id: Option<String>,
    pub batch_job_state: VertexBatchState,
    pub request_metadata: JsonObject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VertexBatchState {
    JobStatePending,
    JobStateFailed,
}

impl VertexBatchLogging {
    #[must_use]
    pub const fn new(
        router: Option<Arc<dyn RouterModelIds>>,
        registry: Option<Arc<dyn ManagedBatchRegistry>>,
        ids: Arc<dyn ResponseIds>,
    ) -> Self {
        Self {
            router,
            registry,
            ids,
        }
    }

    pub fn batch_prediction_jobs_handler(
        &self,
        input: VertexBatchInput<'_>,
        call_log: &mut LoggingState,
    ) -> VertexBatchLogged {
        let outcome = job::handle(self, &input).unwrap_or_else(|error| {
            tracing::error!("Error in batch_prediction_jobs_handler: {error}");
            record::Outcome::Errored(error.to_string())
        });
        if let record::Outcome::Pending(pending) = &outcome {
            pending.model.clone_into(&mut call_log.model);
            let details = &mut call_log.model_call_details;
            details.insert("model".to_owned(), Json::Text(pending.model.clone()));
            details.insert("response_cost".to_owned(), decimal_json(Decimal::ZERO));
            details.insert("batch_id".to_owned(), Json::Text(pending.batch_id.clone()));
        }
        record::logged(&input, outcome, self.ids.new_response_id())
    }

    pub fn store_batch_managed_object(
        &self,
        unified_object_id: &str,
        batch_object: BatchObject,
        request_metadata: &JsonObject,
        is_batch_create: bool,
    ) {
        let Some(registry) = self.registry.as_ref() else {
            tracing::warn!(
                "Managed files hook not available, cannot store batch object for cost tracking"
            );
            return;
        };
        let request = registration::request(
            unified_object_id,
            batch_object,
            request_metadata,
            is_batch_create,
        );
        if let Err(error) = registration::spawn(registry, request) {
            tracing::error!("Error storing batch managed object: {error}");
        }
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn get_actual_model_id_from_router(&self, model_name: &str) -> String {
        let extracted = extract_model_name_from_vertex_path(model_name);
        let Some(router) = self.router.as_ref() else {
            tracing::warn!("Router not available, using constructed model name: {extracted}");
            return extracted;
        };
        match router.model_ids(&extracted).into_iter().next() {
            Some(model_id) => {
                tracing::info!("Found model ID in router: {model_id}");
                model_id
            }
            None => {
                tracing::warn!("Model not found in router, using constructed name: {extracted}");
                extracted
            }
        }
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn extract_model_name_from_vertex_path(vertex_model_path: &str) -> String {
    let named_path =
        vertex_model_path.contains("publishers/") || vertex_model_path.contains("projects/");
    match vertex_model_path.rsplit_once("models/") {
        Some((_, model)) if named_path => model.to_owned(),
        Some(_) | None => vertex_model_path.to_owned(),
    }
}
