use super::fixture::MapFixture;
use super::input::{
    CapabilityCall, Endpoint, Install, MapStep, ReplyStep, ResourceStep, Steps, TransformStep,
    UrlCall, UrlStep, WebsocketStep,
};
use crate::urls::sources::NoSources;
use betterlite_azure::azure_urls::{EnvironmentHeadersRequest, validate_azure_environment};
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
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_config::sections::azure_responses::Settings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_events::RawResponse;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::params::ResponsesOptions;
use betterlite_openai::response_resources::request::{
    InputItemsOrder, ListInputItemsQuery, ResourceOperation, ResourceRequest,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
const DEFAULT_INPUT_ITEMS_LIMIT: i64 = 20;
const PROJECTED: [&str; 3] = ["model", "input", "tools"];
pub(super) fn base_rows(call: &UrlCall) -> Vec<Value> {
    let settings = Settings {
        default_responses_api_version: call.default_api_version.clone(),
    };
    call.steps
        .iter()
        .map(|step| base_row(step, &settings))
        .collect()
}
fn base_row(step: &UrlStep, settings: &Settings) -> Value {
    let key = step.api_key.as_deref().map(SecretString::from);
    let credentials = DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    };
    let validated = validate_azure_environment(EnvironmentHeadersRequest {
        headers: step.headers.clone(),
        api_key: key.as_ref(),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &NoSources,
    });
    let built = complete_url(
        step.api_base.as_deref(),
        step.api_version.as_deref(),
        settings,
    );
    if let (Ok(url), Ok(headers)) = (built, validated) {
        json!({"ok": true, "url": url, "headers": headers, "status": Value::Null})
    } else {
        json!({"ok": false, "url": Value::Null, "headers": Value::Null, "status": Value::Null})
    }
}
pub(super) fn projected_body(step: &TransformStep) -> Result<Value, Box<HelperError>> {
    let body = transform_request(&step.model, &step.input, &step.params, None).or_value_error()?;
    let value = serde_json::to_value(&body)?;
    let projected: serde_json::Map<String, Value> = PROJECTED
        .into_iter()
        .filter_map(|key| value.get(key).map(|found| (key.to_owned(), found.clone())))
        .collect();
    Ok(Value::Object(projected))
}
pub(super) fn websocket_report(call: &Steps<WebsocketStep>) -> Value {
    json!({
        "supports_native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
        "model_in_websocket_url": MODEL_IN_WEBSOCKET_URL,
        "steps": call.steps.iter().map(websocket_step).collect::<Vec<Value>>(),
    })
}
fn websocket_step(step: &WebsocketStep) -> Value {
    match websocket_url(step.api_base.as_deref()) {
        Ok(url) => json!({"ok": true, "url": url, "status": Value::Null}),
        Err(error) => json!({"ok": false, "url": Value::Null, "status": error.http_status()}),
    }
}
fn sent_request(step: &ResourceStep) -> Result<ResourceRequest, ResponsesError> {
    let (base, id) = (&step.api_base, &step.response_id);
    let operation = match step.endpoint {
        Endpoint::List => {
            let query = ListInputItemsQuery {
                after: step.after.clone(),
                before: step.before.clone(),
                include: step.include.clone(),
                limit: Some(step.limit.unwrap_or(DEFAULT_INPUT_ITEMS_LIMIT)),
                order: Some(step.order.unwrap_or(InputItemsOrder::Desc)),
            };
            return list_input_items(base, id, &query);
        }
        Endpoint::Get => ResourceOperation::Get,
        Endpoint::Delete => ResourceOperation::Delete,
        Endpoint::Cancel => ResourceOperation::Cancel,
    };
    resource_request(base, id, operation)
}
pub(super) fn resource_row(step: &ResourceStep) -> Value {
    match sent_request(step) {
        Ok(request) => {
            json!({"ok": true, "url": request.url, "data": request.payload, "status": Value::Null})
        }
        Err(error) => {
            json!({"ok": false, "url": Value::Null, "data": Value::Null, "status": error.http_status()})
        }
    }
}
pub(super) fn reply_row(step: &ReplyStep) -> Value {
    let headers = IndexMap::new();
    let reply = RawResponse {
        status: step.status,
        headers: &headers,
        text: &step.body,
    };
    cancel_reply(&reply).map_or_else(
        |error| json!({"ok": false, "value": Value::Null, "status": error.http_status()}),
        |record| {
            let value = json!({"id": record.id, "created_at": record.created_at,
                "status": record.status, "model": record.model});
            json!({"ok": true, "value": value, "status": Value::Null})
        },
    )
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
    match runtime.block_on(map_params(&mut fixture, &options, drop, &mut bundled)) {
        Ok(value) => json!({
            "ok": true,
            "value": value,
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
        "value": mapped,
        "status": Value::Null,
    })
}
