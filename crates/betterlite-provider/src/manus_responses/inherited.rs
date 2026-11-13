use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::responses_event_records::records::{
    GenericEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, transform_streaming_response,
};
use betterlite_openai::response_params::OPENAI_RESPONSES_API_MIN_MAX_OUTPUT_TOKENS;
use betterlite_openai::response_params::params::supported_openai_params;
use betterlite_openai::response_resources::request::{
    CompactRequest as BaseCompactRequest, InputItemsOrder as BaseOrder, ListInputItemsQuery,
    ResourceOperation, ResourceRequest, compact_request as base_compact_request,
    list_input_items_request, resource_request as base_resource_request,
};
use betterlite_openai::response_resources::response::{decode_record, json_body, response_reply};
use serde_json::Number;

use crate::api::responses_provider_contract::records::{
    CompactRequest, IdRequest, InputItemsOrder, InputItemsRequest, Params, PreparedRequest,
    ResponsesLogging, ResponsesUpstream,
};

use super::error::ManusResponsesError;
use super::json;
use super::reply;
use super::url::ResourceRoute;

const TYPE_FIELD: &str = "type";
const MAX_OUTPUT_TOKENS_FIELD: &str = "max_output_tokens";

pub(super) fn supported_params() -> Vec<String> {
    supported_openai_params()
        .into_iter()
        .map(String::from)
        .collect()
}

pub(super) fn map_params(
    optional_params: &ResponsesApiRequestParams,
) -> Result<Params, ManusResponsesError> {
    let value = serde_json::to_value(optional_params)?;
    let Json::Object(mut fields) = serde_json::from_value::<Json>(value)? else {
        return Err(ManusResponsesError::ParamsShape);
    };
    fields.shift_remove("input");
    fields.shift_remove("model");
    if let Some(Json::Number(number)) = fields.get(MAX_OUTPUT_TOKENS_FIELD)
        && let Some(tokens) = number.as_i64()
    {
        let minimum = i64::from(OPENAI_RESPONSES_API_MIN_MAX_OUTPUT_TOKENS);
        if tokens < minimum {
            fields.insert(
                String::from(MAX_OUTPUT_TOKENS_FIELD),
                Json::Number(Number::from(minimum)),
            );
        }
    }
    Ok(fields.into_iter().collect())
}

pub(super) fn streaming_response(
    chunk: &Json,
) -> Result<ResponsesAPIStreamingResponse, ManusResponsesError> {
    let Json::Object(fields) = chunk else {
        return Err(ManusResponsesError::StreamChunkShape);
    };
    match transform_streaming_response(fields) {
        DecodedStreamResponse::Known { record, .. } => {
            Ok(ResponsesAPIStreamingResponse::Known(record))
        }
        DecodedStreamResponse::Generic(event) => Ok(ResponsesAPIStreamingResponse::Generic(event)),
        DecodedStreamResponse::Lax { fields, .. } => lax_event(fields),
    }
}

fn lax_event(
    fields: indexmap::IndexMap<String, Json>,
) -> Result<ResponsesAPIStreamingResponse, ManusResponsesError> {
    let Some(Json::Text(event_type)) = fields.get(TYPE_FIELD) else {
        return Err(ManusResponsesError::StreamChunkWithoutType);
    };
    let event_type = event_type.clone();
    let mut extra = BTreeMap::new();
    for (name, value) in fields {
        if name != TYPE_FIELD {
            extra.insert(name, value);
        }
    }
    Ok(ResponsesAPIStreamingResponse::Generic(GenericEvent {
        r#type: event_type,
        extra,
    }))
}

fn prepared(resource: &ResourceRequest) -> PreparedRequest {
    PreparedRequest {
        url: resource.url.clone(),
        params: json::from_prompts_object(&resource.payload),
    }
}

pub(super) fn resource_request(
    request: &IdRequest,
    route: ResourceRoute,
) -> Result<PreparedRequest, ManusResponsesError> {
    let operation = match route {
        ResourceRoute::Delete => ResourceOperation::Delete,
        ResourceRoute::Cancel => ResourceOperation::Cancel,
    };
    let resource = base_resource_request(&request.response_id, &request.api_base, operation)?;
    Ok(prepared(&resource))
}

pub(super) fn delete_response(
    response: &ResponsesUpstream,
    _logging: &mut ResponsesLogging,
) -> Result<DeleteResponseResult, ManusResponsesError> {
    let text = String::from_utf8_lossy(&response.body).into_owned();
    let headers = reply::headers_of(response);
    let raw = reply::raw(response, &headers, &text);
    Ok(decode_record::<DeleteResponseResult>(&raw)?)
}

pub(super) fn input_items_request(
    request: &InputItemsRequest,
) -> Result<PreparedRequest, ManusResponsesError> {
    let query = ListInputItemsQuery {
        after: request.query.after.clone(),
        before: request.query.before.clone(),
        include: request.query.include.clone(),
        limit: Some(request.query.limit),
        order: Some(match request.query.order {
            InputItemsOrder::Asc => BaseOrder::Asc,
            InputItemsOrder::Desc => BaseOrder::Desc,
        }),
    };
    let resource = list_input_items_request(&request.response_id, &request.api_base, &query)?;
    Ok(prepared(&resource))
}

pub(super) fn input_items_response(
    response: &ResponsesUpstream,
    _logging: &mut ResponsesLogging,
) -> Result<Params, ManusResponsesError> {
    let text = String::from_utf8_lossy(&response.body).into_owned();
    let headers = reply::headers_of(response);
    let raw = reply::raw(response, &headers, &text);
    match json_body(&raw)? {
        Json::Object(fields) => Ok(fields.into_iter().collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ManusResponsesError::InputItemsShape)
        }
    }
}

pub(super) fn typed_response(
    response: &ResponsesUpstream,
    logging: &mut ResponsesLogging,
) -> Result<ResponsesApiResponse, ManusResponsesError> {
    let text = String::from_utf8_lossy(&response.body).into_owned();
    let headers = reply::headers_of(response);
    let raw = reply::raw(response, &headers, &text);
    let typed = response_reply(&raw)?;
    reply::record_hidden(logging, &response.headers, &typed.additional_headers);
    Ok(typed.response)
}

pub(super) fn compact_request(
    request: &CompactRequest,
) -> Result<PreparedRequest, ManusResponsesError> {
    let input = json::responses_input(&request.input)?;
    let options = json::to_prompts_object(&request.optional_params);
    let base = BaseCompactRequest {
        model: request.model.as_str(),
        input: &input,
        options: &options,
        provider: ProviderId::Manus,
        model_info: None,
    };
    let resource = base_compact_request(&base, request.api_base.as_str())?;
    Ok(prepared(&resource))
}
