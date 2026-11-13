use betterlite_logging::logging_state::LoggingState;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::response_events::{
    RawResponse, ReplyBody, ResponsesReply, transform_response,
};
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::{
    CompactRequest, ListInputItemsQuery, ResourceOperation, ResourceRequest, compact_request,
    list_input_items_request, resource_request,
};
use betterlite_openai::response_resources::response::{delete_response, json_body, response_reply};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::fixtures;
use super::{Call, Endpoint, Probe, invalid_input};
use crate::helpers::{HelperError, HelperResult};

pub(super) fn run(call: &Call) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| invalid_input(error.to_string()))?;
    match call {
        Call::EncodedResponseId(probes)
        | Call::ListDefaultLimitOrder(probes)
        | Call::IncludeCommaQuery(probes)
        | Call::CompactKeepsQuery(probes)
        | Call::CompactReplayInput(probes) => requests(&probes.probes),
        Call::DeleteResult(probes)
        | Call::GetStrictResponse(probes)
        | Call::CancelResponse(probes)
        | Call::CompactLaxResponse(probes)
        | Call::ResourceJsonErrorStatus(probes) => responses(&probes.probes, &redactor),
    }
}

fn requests(probes: &[Probe]) -> HelperResult {
    let mut values = Vec::with_capacity(probes.len());
    for probe in probes {
        values.push(request_value(probe)?);
    }
    Ok(Value::Array(values))
}

fn responses(probes: &[Probe], redactor: &Redactor) -> HelperResult {
    let mut values = Vec::with_capacity(probes.len());
    for probe in probes {
        values.push(response_value(probe, redactor)?);
    }
    Ok(Value::Array(values))
}

fn request_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let api_base = required(probe.api_base.as_deref(), "api_base")?;
    match endpoint(probe)? {
        Endpoint::Compact => {
            let Some(input) = probe.input.as_ref() else {
                return Err(invalid_input(String::from(
                    "a compact request probe needs input",
                )));
            };
            let options = probe.params.clone().unwrap_or_default();
            let request = CompactRequest {
                model: required(probe.model.as_deref(), "model")?,
                input,
                options: &options,
                provider: probe.provider.unwrap_or(ProviderId::Openai),
                model_info: probe.model_info.as_ref(),
            };
            let compacted =
                compact_request(&request, api_base).map_err(|error| response_error(&error))?;
            Ok(request_output(&compacted))
        }
        Endpoint::Delete | Endpoint::Get | Endpoint::Cancel | Endpoint::List => {
            let response_id = required(probe.response_id.as_deref(), "response_id")?;
            let request = item_request(probe, response_id, api_base)?;
            Ok(request_output(&request))
        }
    }
}

fn item_request(
    probe: &Probe,
    response_id: &str,
    api_base: &str,
) -> Result<ResourceRequest, Box<HelperError>> {
    let result = match endpoint(probe)? {
        Endpoint::Delete => resource_request(response_id, api_base, ResourceOperation::Delete),
        Endpoint::Get => resource_request(response_id, api_base, ResourceOperation::Get),
        Endpoint::Cancel => resource_request(response_id, api_base, ResourceOperation::Cancel),
        Endpoint::List => list_input_items_request(response_id, api_base, &query_of(probe)),
        Endpoint::Compact => {
            return Err(invalid_input(String::from(
                "a compact request has no item endpoint",
            )));
        }
    };
    result.map_err(|error| response_error(&error))
}

fn query_of(probe: &Probe) -> ListInputItemsQuery {
    let mut query = ListInputItemsQuery::default();
    query.after.clone_from(&probe.after);
    query.before.clone_from(&probe.before);
    query.include.clone_from(&probe.include);
    if let Some(limit) = probe.limit {
        query.limit = Some(limit);
    }
    if let Some(order) = probe.order {
        query.order = Some(order);
    }
    query
}

fn response_value(probe: &Probe, redactor: &Redactor) -> Result<Value, Box<HelperError>> {
    let endpoint = endpoint(probe)?;
    let body = required(probe.body.as_deref(), "body")?;
    let Some(status) = probe.status else {
        return Err(invalid_input(String::from("a reply probe needs status")));
    };
    let headers = headers_of(probe);
    let reply = RawResponse {
        status,
        headers: &headers,
        text: body,
    };
    Ok(match endpoint {
        Endpoint::Delete => delete_value(&reply),
        Endpoint::Get | Endpoint::Cancel => typed_value(&reply),
        Endpoint::List => list_value(&reply),
        Endpoint::Compact => compact_value(probe, &reply, redactor)?,
    })
}

fn delete_value(reply: &RawResponse<'_>) -> Value {
    match delete_response(reply) {
        Ok(deleted) => json!({
            "id": deleted.id,
            "object": deleted.object,
            "deleted": deleted.deleted,
        }),
        Err(error) => error_value(&error),
    }
}

fn typed_value(reply: &RawResponse<'_>) -> Value {
    match response_reply(reply) {
        Ok(transformed) => json!({
            "id": transformed.response.id,
            "created_at": transformed.response.created_at,
            "status": transformed.response.status,
            "model": transformed.response.model,
            "headers": transformed.headers,
            "additional_headers": transformed.additional_headers,
        }),
        Err(error) => error_value(&error),
    }
}

fn list_value(reply: &RawResponse<'_>) -> Value {
    match json_body(reply) {
        Ok(body) => serde_json::to_value(body).unwrap_or_default(),
        Err(error) => error_value(&error),
    }
}

fn compact_value(
    probe: &Probe,
    reply: &RawResponse<'_>,
    redactor: &Redactor,
) -> Result<Value, Box<HelperError>> {
    let Some(now) = probe.now else {
        return Err(invalid_input(String::from(
            "a compact reply probe needs now",
        )));
    };
    let mut state = fixtures::state(redactor)?;
    let env = fixtures::environment(redactor)?;
    match transform_response(reply, now, &mut state, &env) {
        Ok(transformed) => Ok(compact_output(&transformed, &state, now)),
        Err(OpenAiError::Provider(provider)) => Ok(json!({
            "error": {"status": provider.status_code},
        })),
        Err(_failure) => Ok(json!({
            "error": {"status": null},
        })),
    }
}

fn compact_output(transformed: &ResponsesReply, state: &LoggingState, now: i64) -> Value {
    let (id, created_at) = match &transformed.response {
        ReplyBody::Typed(response) => (Value::String(response.id.clone()), response.created_at),
        ReplyBody::Lax(fields) => (
            fields.get("id").map_or(Value::Null, |given| {
                serde_json::to_value(given).unwrap_or_default()
            }),
            fields
                .get("created_at")
                .and_then(|given| serde_json::to_value(given).ok())
                .and_then(|given| given.as_i64())
                .unwrap_or_default(),
        ),
    };
    let reported = if created_at == now {
        json!("now")
    } else {
        Value::from(created_at)
    };
    let details = &state.model_call_details;
    let recorded = |name: &str| {
        details.get(name).map_or(Value::Null, |given| {
            serde_json::to_value(given).unwrap_or_default()
        })
    };
    json!({
        "id": id,
        "created_at": reported,
        "headers": transformed.headers,
        "additional_headers": transformed.additional_headers,
        "post_call": {
            "original_response": recorded("original_response"),
            "additional_args": recorded("additional_args"),
        },
    })
}

fn headers_of(probe: &Probe) -> IndexMap<String, String> {
    probe
        .headers
        .iter()
        .flatten()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect()
}

fn endpoint(probe: &Probe) -> Result<Endpoint, Box<HelperError>> {
    probe.endpoint.map_or_else(
        || Err(invalid_input(String::from("a probe needs endpoint"))),
        Ok,
    )
}

fn required<'value>(
    value: Option<&'value str>,
    name: &str,
) -> Result<&'value str, Box<HelperError>> {
    match value {
        Some(value) => Ok(value),
        None => Err(invalid_input(format!("a probe needs {name}"))),
    }
}

fn request_output(request: &ResourceRequest) -> Value {
    json!({
        "url": &request.url,
        "data": &request.payload,
    })
}

fn error_value(error: &ResponseResourceError) -> Value {
    json!({
        "error": {"status": error.http_status()},
    })
}

fn response_error(error: &ResponseResourceError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ResponseResourceError"),
        status: error.http_status(),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
