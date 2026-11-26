use betterlite_bedrock::canvas_edits::eligibility::supports_nova_canvas_edit;
use betterlite_bedrock::canvas_edits::environment::{
    USE_MULTIPART_FORM_DATA, get_error_class, validate_environment,
};
use betterlite_bedrock::canvas_edits::error::CanvasEditError;
use betterlite_bedrock::canvas_edits::params::{
    IgnoredResponseFormat, SUPPORTED_OPENAI_PARAMS, map_openai_params,
};
use betterlite_bedrock::canvas_edits::request::{
    CanvasEditRequestInput, transform_image_edit_request,
};
use betterlite_bedrock::canvas_edits::selection::{ImageEditConfig, image_edit_config_for_model};
use betterlite_bedrock::canvas_edits::types::{
    CanvasEditOptions, CanvasEditValue, CanvasImageInput,
};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_openai::api::image_contracts::records::ImageParams;
use serde_json::{Value, json};

use super::fixture::Catalog;
use super::input::{Call, ErrorClassCall, FileSpec, MapCall, ModelsCall, TransformCall};

pub(super) fn observe(call: &Call, model_cost: &ParamMap, catalog: &Catalog) -> Value {
    match call {
        Call::SupportedParams => json!({"supported": SUPPORTED_OPENAI_PARAMS}),
        Call::Map(mapping) => map_answer(mapping),
        Call::Transform(transform) => transform_answer(transform),
        Call::Multipart => json!({"multipart": USE_MULTIPART_FORM_DATA}),
        Call::Url => url_answer(),
        Call::Environment(environment) => {
            json!({"headers": validate_environment(environment.headers.clone())})
        }
        Call::ErrorClass(error) => error_answer(error),
        Call::Eligibility(models) => eligibility_answer(models, model_cost, catalog),
        Call::Selection(models) => {
            let context = catalog.context();
            let selected: Vec<Value> = models
                .models
                .iter()
                .map(|model| {
                    let answer = image_edit_config_for_model(
                        model,
                        model_cost,
                        &context,
                        &mut ModelCostKeys::default(),
                    );
                    match answer {
                        Ok(ImageEditConfig::Stability) => json!({"config": "stability"}),
                        Ok(ImageEditConfig::NovaCanvas) => json!({"config": "nova_canvas"}),
                        Err(error) => json!({"error": error.to_string()}),
                    }
                })
                .collect();
            json!({"selected": selected})
        }
    }
}

fn map_answer(call: &MapCall) -> Value {
    let mapped = map_openai_params(&options_of(&call.options), call.drop_params);
    json!({
        "mapped": rendered(&mapped.options),
        "message": mapped.ignored_response_format.as_ref().map(IgnoredResponseFormat::message),
    })
}

fn transform_answer(call: &TransformCall) -> Value {
    let mut options = options_of(&call.options);
    if let Some(mask) = &call.mask {
        options.insert("mask".to_owned(), mask_value(mask));
    }
    let mapped = map_openai_params(&options, call.drop_params);
    let image = call.image.as_ref().map(image_input);
    let request = CanvasEditRequestInput {
        prompt: call.prompt.as_deref(),
        image: image.as_ref(),
        options: &mapped.options,
    };
    match transform_image_edit_request(&request) {
        Ok(prepared) => json!({"body": prepared.body, "files": prepared.files.len()}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

fn url_answer() -> Value {
    match ImageEditConfig::NovaCanvas.complete_url() {
        Ok(url) => json!({"url": url}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

fn error_answer(call: &ErrorClassCall) -> Value {
    let headers = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let error = get_error_class(call.message.clone(), call.status, headers);
    match &error {
        CanvasEditError::Provider {
            status,
            error: detail,
        } => json!({
            "status": status,
            "message": detail.error.message,
            "headers_present": detail.error.headers.is_some(),
        }),
        CanvasEditError::MissingImage
        | CanvasEditError::TupleFileType
        | CanvasEditError::OutPaintingMask
        | CanvasEditError::InPaintingMask
        | CanvasEditError::UnsupportedTaskType { .. }
        | CanvasEditError::MissingPrompt { .. }
        | CanvasEditError::Field { .. }
        | CanvasEditError::UnsupportedModel { .. }
        | CanvasEditError::CompleteUrl
        | CanvasEditError::Header(_) => json!({
            "status": error.status(),
            "message": error.to_string(),
            "headers_present": false,
        }),
    }
}

fn eligibility_answer(call: &ModelsCall, model_cost: &ParamMap, catalog: &Catalog) -> Value {
    let context = catalog.context();
    let mut eligible: Vec<bool> = Vec::new();
    for model in &call.models {
        eligible.push(supports_nova_canvas_edit(
            model,
            model_cost,
            &context,
            &mut ModelCostKeys::default(),
        ));
    }
    json!({"eligible": eligible})
}

fn options_of(options: &ImageParams) -> CanvasEditOptions {
    options
        .iter()
        .map(|(name, value)| (name.clone(), CanvasEditValue::from(value.clone())))
        .collect()
}

fn rendered(options: &CanvasEditOptions) -> Json {
    Json::Object(
        options
            .iter()
            .filter_map(|(name, value)| value.to_json().map(|json| (name.clone(), json)))
            .collect(),
    )
}

fn image_input(spec: &FileSpec) -> CanvasImageInput {
    match spec {
        FileSpec::Bytes { data } => CanvasImageInput::Bytes(data.as_bytes().to_vec()),
        FileSpec::Text { text } => CanvasImageInput::Text(text.clone()),
        FileSpec::Tuple => CanvasImageInput::Tuple,
    }
}

fn mask_value(spec: &FileSpec) -> CanvasEditValue {
    match spec {
        FileSpec::Bytes { data } => CanvasEditValue::Bytes(data.as_bytes().to_vec()),
        FileSpec::Text { text } => CanvasEditValue::Json(Json::Text(text.clone())),
        FileSpec::Tuple => CanvasEditValue::Tuple,
    }
}
