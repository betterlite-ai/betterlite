use super::fixture::MapFixture;
use super::input::{
    CapabilityCall, Endpoint, Install, MapStep, ReplyStep, ResourceStep, Steps, TransformStep,
    UrlCall, UrlStep, WebsocketStep,
};
use super::sources;
use crate::helpers::HelperError;
use betterlite_azure::responses::capabilities::{
    is_o_series_model, map_params, o_series_map_params, o_series_supported_params, supported_params,
};
use betterlite_azure::responses::decode::cancel_reply;
use betterlite_azure::responses::error::ResponsesError;
use betterlite_azure::responses::request::transform_request;
use betterlite_azure::responses::resources::{list_input_items, resource_request};
use betterlite_azure::responses::url::{
    MODEL_IN_WEBSOCKET_URL, SUPPORTS_NATIVE_WEBSOCKET, complete_url, websocket_url,
};
use betterlite_config::sections::azure_responses::Settings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_events::RawResponse;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::error::ResponseParamsError;
use betterlite_openai::response_params::params::{ResponsesOptions, ResponsesParams};
use betterlite_openai::response_resources::request::{
    InputItemsOrder, ListInputItemsQuery, ResourceOperation, ResourceRequest,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
const DEFAULT_INPUT_ITEMS_LIMIT: i64 = 20;
const PROJECTED: [&str; 3] = ["model", "input", "tools"];
pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
pub(super) fn base_rows(call: &UrlCall) -> Vec<Value> {
    let settings = Settings {
        default_responses_api_version: call.default_api_version.clone(),
    };
    call.urls
        .iter()
        .map(|step| base_row(step, &settings))
        .collect()
}
fn base_row(step: &UrlStep, settings: &Settings) -> Value {
    let key = step.api_key.as_deref().map(SecretString::from);
    let Ok(url) = complete_url(
        step.api_base.as_deref(),
        step.api_version.as_deref(),
        settings,
    ) else {
        return json!({"ok": false, "url": Value::Null, "headers": Value::Null, "status": Value::Null});
    };
    match sources::environment_headers(&step.headers, key.as_ref()) {
        Ok(headers) => json!({"ok": true, "url": url, "headers": headers, "status": Value::Null}),
        Err(_) => {
            json!({"ok": false, "url": Value::Null, "headers": Value::Null, "status": Value::Null})
        }
    }
}
pub(super) fn transform_rows(call: &Steps<TransformStep>) -> Result<Vec<Value>, Box<HelperError>> {
    let mut rows = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        let body = transform_request(&step.model, &step.input, &step.params, None)
            .map_err(|error| invalid_input(error.to_string()))?;
        let value = serde_json::to_value(&body).expect("the request body serializes");
        let projected: serde_json::Map<String, Value> = PROJECTED
            .into_iter()
            .filter_map(|key| value.get(key).map(|found| (key.to_owned(), found.clone())))
            .collect();
        rows.push(Value::Object(projected));
    }
    Ok(rows)
}
pub(super) fn websocket_report(call: &Steps<WebsocketStep>) -> Value {
    let steps: Vec<Value> = call
        .steps
        .iter()
        .map(|step| match websocket_url(step.api_base.as_deref()) {
            Ok(url) => json!({"ok": true, "url": url, "status": Value::Null}),
            Err(error) => json!({"ok": false, "url": Value::Null, "status": error.http_status()}),
        })
        .collect();
    json!({
        "supports_native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
        "model_in_websocket_url": MODEL_IN_WEBSOCKET_URL,
        "steps": steps,
    })
}
pub(super) fn resource_rows(call: &Steps<ResourceStep>) -> Result<Vec<Value>, Box<HelperError>> {
    let mut rows = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        let result = match step.endpoint {
            Endpoint::List => list_request(step),
            Endpoint::Get => {
                resource_request(&step.api_base, &step.response_id, ResourceOperation::Get)
            }
            Endpoint::Delete => {
                resource_request(&step.api_base, &step.response_id, ResourceOperation::Delete)
            }
            Endpoint::Cancel => {
                resource_request(&step.api_base, &step.response_id, ResourceOperation::Cancel)
            }
        };
        rows.push(match result {
            Ok(request) => json!({
                "ok": true,
                "url": request.url,
                "data": serde_json::to_value(&request.payload).expect("the data serializes"),
                "status": Value::Null,
            }),
            Err(error) => json!({
                "ok": false,
                "url": Value::Null,
                "data": Value::Null,
                "status": error.http_status(),
            }),
        });
    }
    Ok(rows)
}
fn list_request(step: &ResourceStep) -> Result<ResourceRequest, ResponsesError> {
    let query = ListInputItemsQuery {
        after: step.after.clone(),
        before: step.before.clone(),
        include: step.include.clone(),
        limit: Some(step.limit.unwrap_or(DEFAULT_INPUT_ITEMS_LIMIT)),
        order: Some(step.order.unwrap_or(InputItemsOrder::Desc)),
    };
    list_input_items(&step.api_base, &step.response_id, &query)
}
pub(super) fn reply_rows(call: &Steps<ReplyStep>) -> Vec<Value> {
    call.steps
        .iter()
        .map(|step| {
            let headers = IndexMap::new();
            let reply = RawResponse {
                status: step.status,
                headers: &headers,
                text: &step.body,
            };
            match cancel_reply(&reply) {
                Ok(response) => json!({
                    "ok": true,
                    "value": {
                        "id": response.id,
                        "created_at": response.created_at,
                        "status": response.status,
                        "model": response.model,
                    },
                    "status": Value::Null,
                }),
                Err(error) => {
                    json!({"ok": false, "value": Value::Null, "status": error.http_status()})
                }
            }
        })
        .collect()
}
pub(super) fn capability_report(runtime: &Runtime, call: &CapabilityCall) -> Value {
    let models = call
        .models
        .iter()
        .map(|model| model_row(runtime, &call.install, model))
        .collect::<Vec<Value>>();
    let map_steps = call
        .map_steps
        .iter()
        .map(|step| map_row(runtime, &call.install, step))
        .collect::<Vec<Value>>();
    let o_series_steps = call
        .o_series_steps
        .iter()
        .map(o_series_row)
        .collect::<Vec<Value>>();
    json!({
        "provider": ProviderId::Azure.to_string(),
        "supported": supported_params().into_iter().collect::<Vec<_>>(),
        "o_series_supported": o_series_supported_params().into_iter().collect::<Vec<_>>(),
        "models": models,
        "map_steps": map_steps,
        "o_series_steps": o_series_steps,
    })
}
fn model_row(runtime: &Runtime, install: &Install, model: &str) -> Value {
    let mut fixture = MapFixture::new(install);
    let o_series = runtime.block_on(is_o_series_model(&mut fixture, model));
    json!({
        "model": model,
        "supported": supported_params().into_iter().collect::<Vec<_>>(),
        "o_series": o_series,
    })
}
fn map_row(runtime: &Runtime, install: &Install, step: &MapStep) -> Value {
    let mut fixture = MapFixture::new(install);
    let mut bundled = BundledReasoningModels::default();
    let options = ResponsesOptions {
        provider: ProviderId::Azure,
        model: step.model.clone(),
        params: step.params.clone(),
    };
    let drop = DropParams {
        global: step.global_drop_params,
        request: step.drop_params,
    };
    map_value(runtime.block_on(map_params(&mut fixture, &options, drop, &mut bundled)))
}
fn map_value(mapped: Result<ResponsesParams, ResponseParamsError>) -> Value {
    match mapped {
        Ok(value) => json!({
            "ok": true,
            "value": serde_json::to_value(value).expect("the mapped params serialize"),
            "status": Value::Null,
        }),
        Err(error) => json!({"ok": false, "value": Value::Null, "status": error.status_code()}),
    }
}
fn o_series_row(step: &MapStep) -> Value {
    let drop = DropParams {
        global: false,
        request: step.drop_params,
    };
    let mapped = o_series_map_params(&step.model, &step.params, drop);
    json!({
        "ok": true,
        "value": serde_json::to_value(mapped).expect("the mapped params serialize"),
        "status": Value::Null,
    })
}
