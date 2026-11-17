mod fixture;
mod view;

mod tests;

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::vertex_predict_logging::{
    VertexEmbedContentInput, VertexPredictInput, VertexPredictLogger,
};
use betterlite_vertex::vertex_relay_metadata::types::{CompletionCost, CompletionCostRequest};
use http::{HeaderMap, StatusCode};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    url_route: String,
    status: u16,
    body: Value,
    #[serde(default)]
    request_body: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Predict,
    PredictImage,
    PredictAudio,
    EmbedContent,
}

#[derive(Clone, Debug)]
struct Recorded {
    model: String,
    provider: &'static str,
    location: Option<String>,
    response_model: Option<String>,
}

struct RecordingCost {
    price: Decimal,
    calls: Mutex<Vec<Recorded>>,
}

impl RecordingCost {
    const fn new(price: Decimal) -> Self {
        Self {
            price,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl CompletionCost for RecordingCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Option<Decimal> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider.as_str(),
                location: request.vertex_location.map(ToOwned::to_owned),
                response_model: request.response.model.clone(),
            });
        Some(self.price)
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cost = Arc::new(RecordingCost::new(Decimal::new(7, 4)));
        let shared = Arc::clone(&cost);
        let logger = VertexPredictLogger::new(shared);
        let logging_obj = fixture::logging_object()?;
        let bytes = serde_json::to_vec(&input.body).or_value_error()?;
        let request_body = request_body(input.request_body.as_ref())?;
        let status = StatusCode::from_u16(input.status).or_value_error()?;
        let headers = HeaderMap::new();
        let kwargs = PassThroughLoggingKwargs::default();
        let outcome = match input.operation {
            Operation::EmbedContent => {
                logger.handle_embed_content_response(VertexEmbedContentInput {
                    logging_obj: &logging_obj,
                    url_route: &input.url_route,
                    status,
                    response_headers: &headers,
                    response_body: &bytes,
                    request_body: request_body.as_ref(),
                    kwargs,
                })
            }
            Operation::Predict | Operation::PredictImage | Operation::PredictAudio => logger
                .handle_predict_response(VertexPredictInput {
                    logging_obj: &logging_obj,
                    url_route: &input.url_route,
                    status,
                    response_headers: &headers,
                    response_body: &bytes,
                    kwargs,
                }),
        };
        outcome.map_or_else(
            |_error| Ok(json!({ "failed": true })),
            |handled| view::outcome_view(&handled, &logging_obj, &cost),
        )
    }
}

fn request_body(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    value
        .map(|body| serde_json::from_value(body.clone()).or_value_error())
        .transpose()
}
