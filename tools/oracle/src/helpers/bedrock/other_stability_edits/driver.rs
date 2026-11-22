use betterlite_bedrock::stability_edits::environment::{
    COMPLETE_URL, USE_MULTIPART_FORM_DATA, get_error_class, validate_environment,
};
use betterlite_bedrock::stability_edits::error::StabilityEditError;
use betterlite_bedrock::stability_edits::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_bedrock::stability_edits::reply::{
    COST_HEADER, ImageEditReply, transform_image_edit_response,
};
use betterlite_bedrock::stability_edits::request::transform_image_edit_request;
use betterlite_bedrock::stability_edits::selection::is_stability_edit_model;
use betterlite_bedrock::stability_edits::types::{EditOptions, EditValue, ImageInput};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageHeaders, ImageHttpResponse, ImageParams,
};
use serde_json::{Value, json};

use super::input::{Call, ErrorClassCall, MapCall, ModelSelectionCall, RequestCall, ResponseCall};
use super::price::Catalog;

// A supplied clock keeps the ImageResponse timestamp out of the comparison.
const NOW_SECONDS: i64 = 1_700_000_000;

pub(super) fn observe(call: &Call) -> Value {
    match call {
        Call::ModelSelection(selection) => selection_answer(selection),
        Call::SupportedParams => json!({"supported": SUPPORTED_OPENAI_PARAMS}),
        Call::Map(mapping) => map_answer(mapping),
        Call::Request(request) => request_answer(request),
        Call::Response(response) => response_answer(response),
        Call::Multipart => json!({"multipart": USE_MULTIPART_FORM_DATA}),
        Call::Url => json!({"url": COMPLETE_URL}),
        Call::Environment(environment) => {
            json!({"headers": validate_environment(environment.headers.clone())})
        }
        Call::ErrorClass(error) => error_class_answer(error),
    }
}

fn selection_answer(call: &ModelSelectionCall) -> Value {
    let selected = call
        .models
        .iter()
        .map(|model| Value::Bool(is_stability_edit_model(Some(model))))
        .collect();
    Value::Array(selected)
}

fn map_answer(call: &MapCall) -> Value {
    let options = json_options(&call.options);
    map_openai_params(&options, &call.model, call.drop_params).map_or_else(
        |_| json!({"unsupported": true}),
        |mapped| json!({"mapped": mapped_json(&mapped)}),
    )
}

fn request_answer(call: &RequestCall) -> Value {
    let options = json_options(&call.options);
    let Ok(mapped) = map_openai_params(&options, &call.model, call.drop_params) else {
        return json!({"unsupported": true});
    };
    let image = call
        .image
        .as_ref()
        .map(|spec| ImageInput::Bytes(spec.value.as_bytes().to_vec()));
    transform_image_edit_request(&call.model, call.prompt.as_deref(), image.as_ref(), &mapped)
        .map_or_else(
            |_| json!({"invalid": true}),
            |request| json!({"body": request.body, "files": request.files.len()}),
        )
}

// Each call reports its own outcome, so one case compares successes and failures side by side.
fn response_answer(call: &ResponseCall) -> Value {
    let upstream = ImageHttpResponse {
        status_code: call.status,
        headers: ImageHeaders::new(),
        body: call.body.as_bytes().to_vec(),
    };
    match transform_image_edit_response(&call.model, &upstream, NOW_SECONDS, &Catalog) {
        Ok(reply) => json!({
            "ok": true,
            "images": image_entries(&reply),
            "cost": reply.additional_headers.get(COST_HEADER),
        }),
        Err(error) => json!({"ok": false, "status": error.status()}),
    }
}

fn error_class_answer(call: &ErrorClassCall) -> Value {
    let headers = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let error = get_error_class(call.message.clone(), call.status, headers);
    match &error {
        StabilityEditError::Provider { error: carried, .. } => json!({
            "status": carried.error.status_code,
            "message": carried.error.message,
            "headers_present": carried.error.headers.is_some(),
        }),
        StabilityEditError::UnsupportedParameter { .. }
        | StabilityEditError::Field { .. }
        | StabilityEditError::ModelInfo { .. }
        | StabilityEditError::Price(_)
        | StabilityEditError::Reply(_)
        | StabilityEditError::Header(_) => json!({
            "status": error.status(),
            "message": error.to_string(),
            "headers_present": false,
        }),
    }
}

fn image_entries(reply: &ImageEditReply) -> Value {
    let mut entries = Vec::new();
    for image in reply.response.data.iter().flatten() {
        if let Some(b64_json) = &image.b64_json {
            entries.push(json!({"b64_json": b64_json}));
        }
    }
    Value::Array(entries)
}

fn json_options(options: &ImageParams) -> EditOptions {
    options
        .iter()
        .map(|(name, value)| (name.clone(), EditValue::from(value.clone())))
        .collect()
}

fn mapped_json(options: &EditOptions) -> Json {
    Json::Object(
        options
            .iter()
            .filter_map(|(name, value)| value.to_json().map(|json| (name.clone(), json)))
            .collect(),
    )
}
