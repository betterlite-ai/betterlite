use betterlite_bedrock::vector_stores::config::capabilities;
use betterlite_bedrock::vector_stores::create::{CreateStep, transform_create_vector_store};
use betterlite_bedrock::vector_stores::error::VectorStoreError;
use betterlite_bedrock::vector_stores::filters::map_openai_params;
use betterlite_bedrock::vector_stores::request::{
    SearchOptionalParams, SearchQuery, SearchRequest, build_search_request,
};
use betterlite_bedrock::vector_stores::response::decode_knowledge_base_reply;
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use super::operations::message_failure;
use super::{Input, RequestScenario, ResponseScenario};
use crate::helpers::HelperError;

#[derive(Clone, Copy)]
pub(super) enum DecodeShape {
    Full,
    Summary,
}

pub(super) fn decode_responses(input: &Input, shape: DecodeShape) -> Value {
    let responses = input
        .responses
        .iter()
        .map(|scenario| decode_outcome(scenario, shape))
        .collect::<Vec<_>>();
    json!({"responses": responses})
}

fn decode_outcome(scenario: &ResponseScenario, shape: DecodeShape) -> Value {
    let decoded = decode_knowledge_base_reply(
        scenario.status,
        &scenario.headers,
        scenario.body.as_bytes(),
        &scenario.query,
    );
    match decoded {
        Ok(response) => match shape {
            DecodeShape::Full => json!({"value": response}),
            DecodeShape::Summary => {
                json!({"ok": true, "count": response.data.map_or(0, |data| data.len())})
            }
        },
        Err(VectorStoreError::Provider { status, .. }) => json!({"error": {"status_code": status}}),
        Err(_) => json!({"failed": true}),
    }
}

pub(super) fn unsupported_create() -> Value {
    json!({
        "request": {"failed": transform_create_vector_store(CreateStep::Request).is_err()},
        "response": {"failed": transform_create_vector_store(CreateStep::Response).is_err()},
        "write": capabilities().endpoints.write,
    })
}

pub(super) fn option_mapping(input: &Input) -> Value {
    let mut results = Vec::with_capacity(input.options.len());
    for scenario in &input.options {
        let mut params = scenario.optional_params.clone();
        let mapped = map_openai_params(&scenario.non_default_params, &mut params).map_or_else(
            |_error| json!({"failed": true}),
            |()| json!({"params": params}),
        );
        results.push(mapped);
    }
    json!({"results": results})
}

pub(super) fn retrieval_configuration(input: &Input) -> Value {
    let calls = input
        .requests
        .iter()
        .map(request_outcome)
        .collect::<Vec<_>>();
    json!({"calls": calls})
}

fn request_outcome(scenario: &RequestScenario) -> Value {
    let Ok(query) = decode_json::<SearchQuery>(&scenario.query) else {
        return json!({"failed": true});
    };
    let Ok(optional) =
        decode_json::<SearchOptionalParams>(&Json::Object(scenario.optional_params.clone()))
    else {
        return json!({"failed": true});
    };
    let extra_body = match &scenario.extra_body {
        Some(Json::Object(fields)) => Some(fields),
        _ => None,
    };
    let prepared = build_search_request(&SearchRequest {
        vector_store_id: &scenario.vector_store_id,
        query: &query,
        optional: &optional,
        api_base: &scenario.api_base,
        litellm_params: &scenario.litellm_params,
        extra_body,
    });
    prepared.map_or_else(
        |_error| json!({"failed": true}),
        |call| json!({"url": call.url, "body": call.body, "query": call.query}),
    )
}

fn decode_json<T>(value: &Json) -> Result<T, Box<HelperError>>
where
    T: serde::de::DeserializeOwned,
{
    match serde_json::to_value(value) {
        Ok(shape) => {
            serde_json::from_value::<T>(shape).map_err(|error| message_failure(&error.to_string()))
        }
        Err(error) => Err(message_failure(&error.to_string())),
    }
}
