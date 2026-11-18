mod build;
mod tests;

use std::sync::Arc;

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, HandlerError, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_vertex::endpoint_target::passthrough::get_vertex_location_from_url;
use betterlite_vertex::vertex_relay_metadata::payload::{
    ResponseLoggingPayloadInput, response_logging_payload,
};
use betterlite_vertex::vertex_relay_metadata::types::ResponseLoggingKwargs;
use betterlite_vertex::vertex_relay_metadata::url_models::{
    custom_provider_from_url, extract_model_from_url,
};

use super::gemini_passthrough_logging::cost::{FactoryCost, PassthroughCompletionCost};
use super::gemini_passthrough_logging::relay::{relay_kwargs, relay_logging, write_relay_logging};

const VERTEX_LOCATION: &str = "vertex_location";

pub struct VertexStreamCollector {
    cost: Arc<dyn PassthroughCompletionCost>,
}

impl CollectedChunksHandler for VertexStreamCollector {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        let vertex_location = get_vertex_location_from_url(call.url_route);
        // The location is stored before the response work, so it is set even when no response can be built.
        if let Some(location) = vertex_location {
            let location = location.to_owned();
            call.logging_obj.edit(&mut |state| {
                state
                    .optional_params
                    .insert(VERTEX_LOCATION.to_owned(), Json::Text(location.clone()));
            });
        }
        let model = call
            .model
            .filter(|name| !name.is_empty())
            .map_or_else(|| extract_model_from_url(call.url_route), str::to_owned);
        let now = call.end_time.timestamp();
        let Some(mut body) = build::vertex_response(call.lines, call.url_route, &model, now)
            .map_err(HandlerError::leaf)?
        else {
            tracing::error!(
                "Unable to build complete streaming response for Vertex passthrough endpoint, not logging..."
            );
            return Ok(HandlerOutcome {
                result: None,
                kwargs: PassThroughLoggingKwargs::default(),
            });
        };
        let kwargs = self.log_response(call, &mut body, &model, vertex_location);
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(body)),
            kwargs,
        })
    }
}

impl VertexStreamCollector {
    // ast-grep-ignore: no-one-line-functions - B8931 constructs the collector through this fixed name
    #[must_use]
    pub fn new(cost: Arc<dyn PassthroughCompletionCost>) -> Self {
        Self { cost }
    }

    // B3638's factory is written against its own logging object and cost trait, so the assembled body is projected onto both.
    fn log_response(
        &self,
        call: &CollectedChunksCall<'_>,
        body: &mut Json,
        model: &str,
        vertex_location: Option<&str>,
    ) -> PassThroughLoggingKwargs {
        let mut record = build::cost_record(body);
        let mut relay = relay_logging(call.logging_obj);
        let kwargs = response_logging_payload(
            &FactoryCost(self.cost.as_ref()),
            ResponseLoggingPayloadInput {
                response: &mut record,
                model,
                kwargs: ResponseLoggingKwargs::default(),
                logging: &mut relay,
                custom_llm_provider: custom_provider_from_url(call.url_route),
                vertex_location,
            },
        );
        write_relay_logging(call.logging_obj, &relay);
        // The factory stamps the call id onto the response it logs, so the returned body carries the same id.
        if let Some(id) = record.id
            && let Json::Object(fields) = body
        {
            fields.insert("id".to_owned(), Json::Text(id));
        }
        relay_kwargs(&PassThroughLoggingKwargs::default(), kwargs)
    }
}
