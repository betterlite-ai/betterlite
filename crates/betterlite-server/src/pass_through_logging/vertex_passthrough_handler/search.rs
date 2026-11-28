use std::collections::BTreeMap;

use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_core::routes::CallType;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_vertex::vertex_discovery_search::records::payload::transform_search_response;
use betterlite_vertex::vertex_rag_store::records::payload::Reply;
use http::HeaderMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{VertexPassthroughError, VertexPassthroughInput};
use crate::pass_through_logging::gemini_passthrough_logging::cost::{
    CostRequest, CostResponse, PassthroughCompletionCost,
};

const SEARCH_MODEL: &str = "vertex_ai/search_api";

// transform_search_vector_store_response and the cost it books under the search_api model.
pub(super) fn search(
    cost: &dyn PassthroughCompletionCost,
    input: VertexPassthroughInput<'_>,
    vertex_location: Option<&str>,
) -> Result<HandlerOutcome, VertexPassthroughError> {
    let headers = header_map(input.response_headers);
    let details = search_details(input.call_log);
    let found = transform_search_response(
        &Reply {
            status: input.status.as_u16(),
            headers: &headers,
            body: input.response_body,
        },
        &details,
    )?;
    let body = search_json(&found)?;
    let response_cost = search_cost(cost, &body, vertex_location)?;
    write_details(input.call_log, response_cost)?;
    Ok(HandlerOutcome {
        result: Some(PassThroughResponse::VertexSearchWrapper(
            PassThroughResponseObject { response: body },
        )),
        kwargs: search_kwargs(input.kwargs, response_cost),
    })
}

fn search_json(found: &VectorStoreSearchResponse) -> Result<Json, VertexPassthroughError> {
    serde_json::to_value(found)
        .and_then(serde_json::from_value)
        .map_err(|source| VertexPassthroughError::Encode { source })
}

fn search_cost(
    cost: &dyn PassthroughCompletionCost,
    body: &Json,
    vertex_location: Option<&str>,
) -> Result<Decimal, VertexPassthroughError> {
    cost.completion_cost(&CostRequest {
        response: CostResponse::Completion(body),
        model: SEARCH_MODEL,
        custom_llm_provider: "vertex_ai",
        vertex_location,
        call_type: CallType::VectorStoreSearch,
    })
    .map_err(|source| VertexPassthroughError::Cost {
        model: SEARCH_MODEL.to_owned(),
        source,
    })
}

// setdefault("litellm_params", {}) then base_model: a caller's own litellm_params entries stay.
fn write_details(
    call_log: &mut LoggingState,
    response_cost: Decimal,
) -> Result<(), VertexPassthroughError> {
    let mut params = match call_log.model_call_details.get("litellm_params").cloned() {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VertexPassthroughError::Detail {
                field: "litellm_params",
            });
        }
    };
    params.insert("base_model".to_owned(), Json::Text(SEARCH_MODEL.to_owned()));
    call_log
        .model_call_details
        .insert("litellm_params".to_owned(), Json::Object(params));
    call_log
        .model_call_details
        .insert("response_cost".to_owned(), decimal_json(response_cost));
    Ok(())
}

fn search_kwargs(
    kwargs: PassThroughLoggingKwargs,
    response_cost: Decimal,
) -> PassThroughLoggingKwargs {
    let mut entries = kwargs.entries;
    entries.insert("response_cost".to_owned(), decimal_json(response_cost));
    entries.insert("model".to_owned(), Json::Text(SEARCH_MODEL.to_owned()));
    PassThroughLoggingKwargs { entries }
}

fn header_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}

// The search transform reads the caller's query back from what the call logged.
fn search_details(call_log: &LoggingState) -> BTreeMap<String, Json> {
    call_log
        .model_call_details
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
