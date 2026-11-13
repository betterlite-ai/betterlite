use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_provider::api::responses_provider_contract::config::ResponsesProviderConfig as _;
use betterlite_provider::api::responses_provider_contract::error::ResponsesContractError;
use betterlite_provider::api::responses_provider_contract::records::{
    CreateRequest, Headers, IdRequest, Params, ResponsesLogging, ResponsesUpstream,
};
use betterlite_provider::manus_responses::ManusResponsesConfig;
use betterlite_provider::manus_responses::error::ManusResponsesError;
use serde_json::{Value, json};

use super::fixture;
use super::{Creates, Gets, Replies, ReplyProbe, Urls};

fn create_body(
    config: &ManusResponsesConfig,
    model: &str,
    input: &Json,
    optional_params: &Params,
) -> Option<Params> {
    let request = CreateRequest {
        model: model.to_owned(),
        input: input.clone(),
        optional_params: optional_params.clone(),
        litellm_params: Params::new(),
        headers: Headers::new(),
    };
    config.transform_create_request(&request).ok()
}

pub(super) fn urls(call: &Urls) -> Value {
    let config = fixture::config(&call.environment, &call.sdk, 0);
    let evaluated: Vec<Value> = call
        .probes
        .iter()
        .map(|probe| {
            config
                .complete_url(probe.api_base.as_deref(), &Params::new())
                .map_or_else(|_| json!({"raised": true}), |url| json!({"url": url}))
        })
        .collect();
    json!({"urls": evaluated})
}

pub(super) fn creates(call: &Creates) -> Value {
    let config = fixture::config(&call.environment, &call.sdk, 0);
    let bodies: Vec<Value> = call
        .probes
        .iter()
        .map(|probe| {
            create_body(&config, &probe.model, &probe.input, &probe.optional_params)
                .map_or_else(|| json!({"raised": true}), |body| json!({"body": body}))
        })
        .collect();
    json!({"requests": bodies})
}

fn upstream_response(probe: &ReplyProbe) -> ResponsesUpstream {
    ResponsesUpstream {
        status_code: probe.status,
        headers: probe
            .headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
            .collect(),
        body: probe.body.as_bytes().to_vec(),
    }
}

fn transform_failure(error: &ManusResponsesError) -> Value {
    match error {
        ManusResponsesError::Contract(ResponsesContractError::BaseLlm(inner)) => {
            json!({"error": {"status": inner.status_code}})
        }
        ManusResponsesError::Contract(ResponsesContractError::ApiBaseRequired)
        | ManusResponsesError::MissingApiKey
        | ManusResponsesError::Environment { .. }
        | ManusResponsesError::ParamNotText { .. }
        | ManusResponsesError::InputShape
        | ManusResponsesError::ExtraBodyShape
        | ManusResponsesError::ResponseId { .. }
        | ManusResponsesError::Request(_)
        | ManusResponsesError::Resource(_)
        | ManusResponsesError::Json(_)
        | ManusResponsesError::ParamsShape
        | ManusResponsesError::StreamChunkShape
        | ManusResponsesError::StreamChunkWithoutType
        | ManusResponsesError::InputItemsShape => json!({"raised": true}),
    }
}

fn recorded(logging: &ResponsesLogging, name: &str) -> Value {
    logging
        .model_call_details
        .get(name)
        .map_or(Value::Null, |value| {
            serde_json::to_value(value).unwrap_or(Value::Null)
        })
}

fn reply_fields(response: &ResponsesApiResponse, logging: &ResponsesLogging) -> Value {
    let usage = response.usage.as_ref().map_or(Value::Null, |usage| {
        json!({
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens,
        })
    });
    json!({
        "id": response.id,
        "created_at": response.created_at,
        "reasoning": response.reasoning,
        "text": response.text,
        "output": response.output,
        "usage": usage,
        "headers": recorded(logging, "headers"),
        "additional_headers": recorded(logging, "additional_headers"),
        "post_call": {
            "original_response": recorded(logging, "original_response"),
            "additional_args": recorded(logging, "additional_args"),
        },
    })
}

pub(super) fn replies(call: &Replies) -> Value {
    let config = fixture::config(&call.environment, &call.sdk, call.now);
    let evaluated: Vec<Value> = call
        .probes
        .iter()
        .map(|probe| {
            let upstream = upstream_response(probe);
            let mut create_logging = ResponsesLogging::default();
            let created =
                config.transform_create_response("manus-1.6", &upstream, &mut create_logging);
            let mut get_logging = ResponsesLogging::default();
            let retrieved = config.transform_get_response(&upstream, &mut get_logging);
            json!({
                "create": created.map_or_else(
                    |error| transform_failure(&error),
                    |response| reply_fields(&response, &create_logging),
                ),
                "retrieve": retrieved.map_or_else(
                    |error| transform_failure(&error),
                    |response| reply_fields(&response, &get_logging),
                ),
            })
        })
        .collect();
    json!({"replies": evaluated})
}

pub(super) fn gets(call: &Gets) -> Value {
    let config = fixture::config(&BTreeMap::new(), &fixture::empty_sdk(), 0);
    let prepared: Vec<Value> = call
        .probes
        .iter()
        .map(|probe| {
            let request = IdRequest {
                response_id: probe.response_id.clone(),
                api_base: probe.api_base.clone(),
                litellm_params: Params::new(),
                headers: Headers::new(),
            };
            config.transform_get_request(&request).map_or_else(
                |_error| json!({"raised": true}),
                |prepared| json!({"url": prepared.url, "data": prepared.params}),
            )
        })
        .collect();
    json!({"requests": prepared})
}
