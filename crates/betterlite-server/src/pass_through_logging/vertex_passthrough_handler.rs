use std::sync::Arc;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::error::RedactionError;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_vertex::endpoint_target::passthrough::get_vertex_location_from_url;
use betterlite_vertex::model_policy::error::VertexAiError;
use betterlite_vertex::partner_response_config::error::PartnerResponseError;
use betterlite_vertex::studio_veo_operation::error::StudioVeoOperationError;
use betterlite_vertex::vertex_discovery_search::error::VertexDiscoverySearchError;
use betterlite_vertex::vertex_relay_metadata::error::VertexRelayError;
use betterlite_vertex::vertex_relay_metadata::routes::is_interactions_route;
use chrono::{DateTime, FixedOffset};
use http::{HeaderMap, StatusCode};
use thiserror::Error;

use self::counter::PartnerTools;
use self::predict::ReplyKind;
use crate::pass_through_logging::gemini_passthrough_logging::cost::{
    PassthroughCompletionCost, PassthroughCostError,
};
use crate::pass_through_logging::vertex_batch_logging::VertexBatchLogging;
use crate::pass_through_logging::vertex_predict_logging::VertexPredictError;
use crate::pass_through_logging::vertex_predict_logging::VertexPredictLogger;

mod batch;
mod counter;
mod generate;
mod interactions;
mod predict;
mod raw_predict;
mod response_body;
mod search;
mod tests;
mod video;

pub struct VertexPassthroughInput<'input> {
    pub logging_obj: &'input StreamLoggingObject,
    pub url_route: &'input str,
    pub result: &'input str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub status: StatusCode,
    pub response_headers: &'input HeaderMap,
    pub response_body: &'input [u8],
    pub request_body: Option<&'input JsonMap>,
    pub call_log: &'input mut LoggingState,
    pub kwargs: PassThroughLoggingKwargs,
}

#[derive(Debug, Error)]
pub enum VertexPassthroughError {
    #[error(transparent)]
    Interactions(#[from] VertexRelayError),
    #[error("the predictLongRunning reply could not be built: {source}")]
    Video {
        #[from]
        source: StudioVeoOperationError,
    },
    #[error("the generateContent reply could not be decoded: {source}")]
    Generate {
        #[from]
        source: VertexAiError,
    },
    #[error(transparent)]
    Predict(#[from] VertexPredictError),
    #[error("the rawPredict reply is not valid JSON: {source}")]
    RawBody {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {publisher} partner reply could not be transformed: {source}")]
    Partner {
        publisher: &'static str,
        #[source]
        source: Box<PartnerResponseError<EngineError>>,
    },
    #[error("the Vertex search reply could not be decoded: {source}")]
    Search {
        #[from]
        source: VertexDiscoverySearchError,
    },
    #[error("the response cost of model {model} could not be resolved: {source}")]
    Cost {
        model: String,
        #[source]
        source: PassthroughCostError,
    },
    #[error("the logged {kind} is not a JSON object")]
    Shape { kind: &'static str },
    #[error("the call detail {field} is not a JSON object")]
    Detail { field: &'static str },
    #[error("the request body could not be read as the Vertex JSON type: {source}")]
    RequestBody {
        #[source]
        source: serde_json::Error,
    },
    #[error("the logged Vertex reply could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vertex passthrough redactor could not be built: {source}")]
    Redactor {
        #[from]
        source: RedactionError,
    },
    #[error("the Vertex passthrough logging tools could not be built")]
    Tools,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VertexBranch {
    Interactions,
    Video,
    GenerateContent,
    EmbedContent,
    Predict,
    RawPredict,
    Search,
    BatchPredictionJobs,
}

// LiteLLM's tests run in this order against the whole route string, project and model names included.
#[must_use]
pub fn select_vertex_branch(url_route: &str) -> Option<VertexBranch> {
    if is_interactions_route(url_route) {
        return Some(VertexBranch::Interactions);
    }
    if url_route.contains("predictLongRunning") {
        return Some(VertexBranch::Video);
    }
    if url_route.contains("generateContent") {
        return Some(VertexBranch::GenerateContent);
    }
    if url_route.contains("embedContent") || url_route.contains("batchEmbedContents") {
        return Some(VertexBranch::EmbedContent);
    }
    if url_route.contains("predict") {
        return Some(VertexBranch::Predict);
    }
    if url_route.contains("rawPredict") || url_route.contains("streamRawPredict") {
        return Some(VertexBranch::RawPredict);
    }
    if url_route.contains("search") {
        return Some(VertexBranch::Search);
    }
    if url_route.contains("batchPredictionJobs") {
        return Some(VertexBranch::BatchPredictionJobs);
    }
    None
}

pub struct VertexPassthroughHandler {
    predict: VertexPredictLogger,
    batch: VertexBatchLogging,
    cost: Arc<dyn PassthroughCompletionCost>,
    tools: Option<PartnerTools>,
}

impl VertexPassthroughHandler {
    #[must_use]
    pub fn new(
        predict: VertexPredictLogger,
        batch: VertexBatchLogging,
        cost: Arc<dyn PassthroughCompletionCost>,
    ) -> Self {
        Self {
            predict,
            batch,
            cost,
            tools: counter::partner_tools(),
        }
    }

    pub fn vertex_passthrough_handler(
        &self,
        input: VertexPassthroughInput<'_>,
    ) -> Result<HandlerOutcome, VertexPassthroughError> {
        let vertex_location = get_vertex_location_from_url(input.url_route);
        insert_location(input.logging_obj, vertex_location);
        match select_vertex_branch(input.url_route) {
            Some(VertexBranch::Interactions) => {
                interactions::interactions(self.cost.as_ref(), input, vertex_location)
            }
            Some(VertexBranch::Video) => video::video(self.cost.as_ref(), input, vertex_location),
            Some(VertexBranch::GenerateContent) => {
                generate::generate_content(self.cost.as_ref(), input, vertex_location)
            }
            Some(VertexBranch::EmbedContent) => {
                predict::reply(ReplyKind::Embed, &self.predict, input)
            }
            Some(VertexBranch::Predict) => predict::reply(ReplyKind::Predict, &self.predict, input),
            Some(VertexBranch::RawPredict) => raw_predict::raw_predict(
                self.cost.as_ref(),
                self.tools.as_ref(),
                input,
                vertex_location,
            ),
            Some(VertexBranch::Search) => {
                search::search(self.cost.as_ref(), input, vertex_location)
            }
            Some(VertexBranch::BatchPredictionJobs) => {
                batch::batch_prediction_jobs(&self.batch, input)
            }
            None => Ok(HandlerOutcome {
                result: None,
                kwargs: input.kwargs,
            }),
        }
    }
}

// Every route records its region, whether or not a branch claims it.
fn insert_location(logging_obj: &StreamLoggingObject, location: Option<&str>) {
    let Some(found) = location else {
        return;
    };
    logging_obj.edit(&mut |state| {
        state
            .optional_params
            .insert("vertex_location".to_owned(), Json::Text(found.to_owned()));
    });
}
