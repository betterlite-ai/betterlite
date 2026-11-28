use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::cost_usage::CostUsage;
use betterlite_vertex::gemini_response::complete::{
    CompleteRequest, CompleteResponse, RawResponse, transform_response,
};
use betterlite_vertex::gemini_response_parts::projection::CallMode;
use betterlite_vertex::model_policy::error::VertexAiError;
use betterlite_vertex::vertex_relay_metadata::payload::{
    ResponseLoggingPayloadInput, response_logging_payload,
};
use betterlite_vertex::vertex_relay_metadata::types::{CompletionResponse, ResponseLoggingKwargs};
use betterlite_vertex::vertex_relay_metadata::url_models::{
    custom_provider_from_url, extract_model_from_url,
};
use http::HeaderMap;

use super::response_body::set_body_id;
use super::{VertexPassthroughError, VertexPassthroughInput};
use crate::pass_through_logging::gemini_passthrough_logging::cost::{
    FactoryCost, PassthroughCompletionCost,
};
use crate::pass_through_logging::gemini_passthrough_logging::relay::{RelayLogging, relay_kwargs};
use crate::pass_through_logging::gemini_passthrough_logging::reply;

// VertexGeminiConfig.transform_response with the endpoint's dummy message and empty request parameters.
pub(super) fn generate_content(
    cost: &dyn PassthroughCompletionCost,
    input: VertexPassthroughInput<'_>,
    vertex_location: Option<&str>,
) -> Result<HandlerOutcome, VertexPassthroughError> {
    let kwargs = input.kwargs;
    let model = extract_model_from_url(input.url_route);
    let provider = custom_provider_from_url(input.url_route);
    let response = decode_reply(
        input.response_headers,
        input.response_body,
        &model,
        input.end_time.timestamp(),
    )?;
    let mut body = reply::logged_response(&response, input.end_time.timestamp())
        .map_err(|source| VertexPassthroughError::Encode { source })?;
    let mut relay = RelayLogging::project(input.logging_obj);
    let mut completion = completion_of(&response);
    let logged = response_logging_payload(
        &FactoryCost(cost),
        ResponseLoggingPayloadInput {
            response: &mut completion,
            model: &model,
            kwargs: ResponseLoggingKwargs::default(),
            logging: &mut relay.logging,
            custom_llm_provider: provider,
            vertex_location,
        },
    );
    relay.commit();
    // The factory stamps the call id on the response object, which is the body later logged.
    set_body_id(&mut body, &completion.id.unwrap_or_default())?;
    Ok(HandlerOutcome {
        result: Some(PassThroughResponse::ModelResponse(body)),
        kwargs: relay_kwargs(&kwargs, logged),
    })
}

fn decode_reply(
    headers: &HeaderMap,
    body: &[u8],
    model: &str,
    now: i64,
) -> Result<CompleteResponse, VertexAiError> {
    let mut ids = uuid::Uuid::new_v4;
    transform_response(
        &RawResponse { headers, body },
        &CompleteRequest {
            model,
            now,
            call_mode: CallMode::Tools,
        },
        &mut ids,
    )
}

fn completion_of(response: &CompleteResponse) -> CompletionResponse {
    CompletionResponse {
        id: Some(response.id.clone()),
        model: Some(response.model.clone()),
        usage: Some(CostUsage::from(&response.usage)),
    }
}
