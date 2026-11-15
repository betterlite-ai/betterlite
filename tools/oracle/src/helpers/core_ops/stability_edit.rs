use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_core::api::stability::edit::error::StabilityEditError;
use betterlite_core::api::stability::edit::reply::{
    COST_HEADER, ModelInfoLookup, transform_image_edit_response,
};
use betterlite_core::api::stability::edit::request::transform_image_edit_request;
use betterlite_core::api::stability::edit::types::{EditOptions, EditValue};
use betterlite_core::api::stability::edit::{
    StabilityImageEditConfig,
    // ast-grep-ignore: no-long-names
    get_stability_image_edit_config as edit_config,
};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageHeaders, ImageHttpResponse, ImageParams,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

// internally tagged, so each variant's payload struct sees only its own fields
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Map(MapCall),
    Url(UrlCall),
    Environment(EnvironmentCall),
    Request(RequestCall),
    Response(ResponseCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapCall {
    model: String,
    #[serde(default)]
    options: ImageParams,
    #[serde(default)]
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    model: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    litellm_params: ImageParams,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    model: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    headers: ImageHeaders,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    model: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    image: Option<Vec<FileSpec>>,
    #[serde(default)]
    options: ImageParams,
    #[serde(default)]
    files: indexmap::IndexMap<String, Vec<FileSpec>>,
    #[serde(default)]
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseCall {
    model: String,
    status: u16,
    #[serde(default)]
    headers: ImageHeaders,
    body: String,
    created: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSpec {
    content_base64: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let values: Result<Vec<Value>, Box<HelperError>> =
            input.calls.iter().map(run_call).collect();
        Ok(Value::Array(values?))
    }
}

fn run_call(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Map(call) => Ok(map_answer(call)),
        Call::Url(call) => Ok(url_answer(call)),
        Call::Environment(call) => Ok(environment_answer(call)),
        Call::Request(call) => request_answer(call),
        Call::Response(call) => response_answer(call),
    }
}

fn map_answer(call: &MapCall) -> Value {
    let options = json_options(&call.options);
    match StabilityImageEditConfig::map_openai_params(&options, &call.model, call.drop_params) {
        Ok(mapped) => json!({"mapped": mapped_json(&mapped)}),
        Err(_) => json!({"unsupported": true}),
    }
}

fn url_answer(call: &UrlCall) -> Value {
    let config = edit_config(&call.model, &call.environment);
    json!({
        "url": config.get_complete_url(call.api_base.as_deref(), &call.litellm_params, &call.model)
    })
}

fn environment_answer(call: &EnvironmentCall) -> Value {
    let config = edit_config(&call.model, &call.environment);
    let explicit = call.api_key.as_deref().map(SecretString::from);
    match config.validate_environment(call.headers.clone(), explicit.as_ref()) {
        Ok(validated) => json!({"headers": validated}),
        Err(_) => json!({"missing_key": true}),
    }
}

fn request_answer(call: &RequestCall) -> Result<Value, Box<HelperError>> {
    let options = json_options(&call.options);
    let mut mapped = match StabilityImageEditConfig::map_openai_params(
        &options,
        &call.model,
        call.drop_params,
    ) {
        Ok(mapped) => mapped,
        Err(_) => return Ok(json!({"unsupported": true})),
    };
    for (name, specs) in &call.files {
        let parts = specs.iter().map(file_of).collect::<Result<Vec<_>, _>>()?;
        mapped.insert(name.clone(), EditValue::Files(parts));
    }
    let images = image_files(call.image.as_deref())?;
    match transform_image_edit_request(call.prompt.as_deref(), images.as_deref(), &mapped) {
        Ok(request) => Ok(json!({
            "data": request.data,
            "files": files_value(&request.files),
        })),
        Err(_) => Ok(json!({"invalid": true})),
    }
}

fn response_answer(call: &ResponseCall) -> Result<Value, Box<HelperError>> {
    let upstream = ImageHttpResponse {
        status_code: call.status,
        headers: call.headers.clone(),
        body: call.body.as_bytes().to_vec(),
    };
    match transform_image_edit_response(&call.model, &upstream, call.created, &Catalog) {
        Ok(reply) => Ok(json!({
            "response": reply.response,
            "cost": reply.additional_headers.get(COST_HEADER),
        })),
        Err(error) => Err(reply_error(&error)),
    }
}

fn json_options(options: &ImageParams) -> EditOptions {
    options
        .iter()
        .map(|(name, value)| (name.clone(), EditValue::Json(value.clone())))
        .collect()
}

fn mapped_json(options: &EditOptions) -> ImageParams {
    options
        .iter()
        .filter_map(|(name, value)| match value {
            EditValue::Json(json) => Some((name.clone(), json.clone())),
            EditValue::Files(_) => None,
        })
        .collect()
}

fn file_of(spec: &FileSpec) -> Result<ImageFile, Box<HelperError>> {
    let content = BASE64.decode(&spec.content_base64).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(ImageFile {
        filename: None,
        content,
        content_type: None,
    })
}

fn image_files(specs: Option<&[FileSpec]>) -> Result<Option<Vec<ImageFile>>, Box<HelperError>> {
    specs
        .map(|specs| specs.iter().map(file_of).collect())
        .transpose()
}

fn files_value(files: &ImageFiles) -> Value {
    Value::Array(
        files
            .iter()
            .map(|(name, file)| {
                json!({
                    "name": name,
                    "content_base64": BASE64.encode(&file.content),
                })
            })
            .collect(),
    )
}

struct Catalog;

#[derive(Debug, thiserror::Error)]
#[error("the model {model} is not mapped")]
struct NotMapped {
    model: String,
}

impl ModelInfoLookup for Catalog {
    type Error = NotMapped;

    fn model_info(&self, model: &str) -> Result<JsonObject, NotMapped> {
        price_record(model).ok_or_else(|| NotMapped {
            model: model.to_owned(),
        })
    }
}

// B2225 reads its catalog prices from the same bundled asset, so the helper resolves the entry there
fn price_record(model: &str) -> Option<JsonObject> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let canonical = catalog.lookup.get(model)?;
    let entry = catalog.entries.get(canonical.as_ref())?;
    let prices = serde_json::to_value(&entry.prices).ok()?;
    serde_json::from_value(prices).ok()
}

fn reply_error(error: &StabilityEditError<NotMapped>) -> Box<HelperError> {
    let status = match error {
        StabilityEditError::Reply(reply) => Some(reply.status()),
        StabilityEditError::MissingApiKey
        | StabilityEditError::UnsupportedParameter { .. }
        | StabilityEditError::RequestField { .. }
        | StabilityEditError::MissingFile { .. }
        | StabilityEditError::CostValue(_)
        | StabilityEditError::ModelInfo(_) => None,
    };
    Box::new(HelperError {
        class: "BaseLLMException".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
