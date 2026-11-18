use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::DecodedJson;
use betterlite_core::api::request_form::annotations::{FormAnnotation, FormAnnotations};
use betterlite_core::api::request_form::coercion::coerce_numeric_form_fields;
use betterlite_core::api::request_form::convert::{
    FileDataValue, convert_upload_files_to_file_data,
};
use betterlite_core::api::request_form::file_size::{UploadSizeCheck, check_file_size_under_limit};
use betterlite_core::api::request_form::metadata::{
    MetadataTree, MetadataValue, extract_nested_form_metadata,
};
use betterlite_core::api::request_form::values::{
    FormFields, FormPart, FormValue, PartValue, UploadFile, get_form_data,
};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    operation: String,
    #[serde(default)]
    fields: Vec<(String, String)>,
    #[serde(default)]
    uploads: Vec<UploadInput>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
    #[serde(default)]
    method: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UploadInput {
    name: String,
    filename: String,
    content_type: Option<String>,
    content: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut parts = input
            .fields
            .iter()
            .map(|(name, value)| FormPart {
                name: name.clone(),
                value: PartValue::Text(value.clone()),
            })
            .collect::<Vec<_>>();
        parts.extend(input.uploads.iter().map(|upload| FormPart {
            name: upload.name.clone(),
            value: PartValue::Upload(UploadFile {
                filename: upload.filename.clone(),
                content_type: upload.content_type.clone(),
                content: upload.content.as_bytes().to_vec(),
                size: Some(u64::try_from(upload.content.len()).unwrap_or(u64::MAX)),
            }),
        }));
        let fields =
            get_form_data(&betterlite_core::api::request_form::values::FormParts { items: parts })
                .map_err(|error| failure("FormError", error.to_string()))?;
        match input.operation.as_str() {
            "numeric_form_coercion" => numeric(input, fields),
            "nested_form_metadata" => Ok(metadata(input, &fields)),
            "upload_size_unset" => upload_size(input),
            "request_body_dispatch" => body_dispatch(input),
            _other => file_values(&fields),
        }
    }
}

fn upload_size(input: &Input) -> HelperResult {
    let mut request_data = betterlite_models::model_registration::JsonObject::new();
    request_data.insert(
        "model".to_owned(),
        betterlite_models::decisions::Json::Text("whisper".to_owned()),
    );
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut request_data,
        file_size: input.file_size,
        router_names: &std::collections::BTreeSet::new(),
        router: None,
        limits: &ApiLimits::default(),
    })
    .map_err(|error| failure("ValueError", error.to_string()))?;
    let size = match request_data.get("metadata") {
        Some(betterlite_models::decisions::Json::Object(metadata)) => {
            metadata.get("file_size_in_mb")
        }
        _other => None,
    };
    let size = size
        .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
        .unwrap_or(Value::Null);
    Ok(json!({"proceeded": outcome.proceeded, "file_size_in_mb": size}))
}

fn body_dispatch(input: &Input) -> HelperResult {
    match betterlite_server::api::request_form::get_request_body(
        &input.method,
        &input.content_type,
        input.body.as_bytes(),
        &ApiLimits::default(),
    ) {
        Ok(betterlite_server::api::request_form::RequestFormBody::Empty) => Ok(json!({})),
        Ok(betterlite_server::api::request_form::RequestFormBody::Json(DecodedJson::Parsed {
            body,
            ..
        })) => serde_json::to_value(body).map_err(|error| failure("ValueError", error.to_string())),
        Ok(betterlite_server::api::request_form::RequestFormBody::Json(
            DecodedJson::Unreadable(_error),
        )) => Ok(json!({})),
        Ok(betterlite_server::api::request_form::RequestFormBody::Form(fields)) => {
            file_values(&fields)
        }
        Err(betterlite_server::api::request_form::RequestFormError::UnsupportedContentType(
            content_type,
        )) => Err(failure(
            "ValueError",
            format!("Unsupported content type: {content_type}"),
        )),
        Err(error) => Err(failure("ValueError", error.to_string())),
    }
}

fn numeric(input: &Input, fields: FormFields) -> HelperResult {
    let annotations: FormAnnotations = input
        .annotations
        .iter()
        .map(|(name, kind)| {
            let annotation = match kind.as_str() {
                "int" => FormAnnotation::Int,
                "float" => FormAnnotation::Float,
                "optional_int" => {
                    FormAnnotation::Union(vec![FormAnnotation::Int, FormAnnotation::NoneType])
                }
                "optional_float" => {
                    FormAnnotation::Union(vec![FormAnnotation::Float, FormAnnotation::NoneType])
                }
                _other => FormAnnotation::Other,
            };
            (name.clone(), annotation)
        })
        .collect();
    let numeric =
        betterlite_core::api::request_form::annotations::numeric_form_fields(&annotations);
    Ok(form_values(&coerce_numeric_form_fields(&fields, &numeric)))
}

fn metadata(input: &Input, fields: &FormFields) -> Value {
    let (tree, _warnings) = extract_nested_form_metadata(
        fields,
        input.prefix.as_deref().unwrap_or("litellm_metadata["),
    );
    metadata_tree(&tree)
}

fn metadata_tree(tree: &MetadataTree) -> Value {
    let entries = tree
        .iter()
        .map(|(key, value)| {
            let value = match value {
                MetadataValue::Leaf(value) => form_value(value),
                MetadataValue::Object(child) => metadata_tree(child),
            };
            (key.clone(), value)
        })
        .collect::<serde_json::Map<_, _>>();
    Value::Object(entries)
}

fn form_values(fields: &FormFields) -> Value {
    let entries = fields
        .iter()
        .map(|(key, value)| (key.clone(), form_value(value)))
        .collect::<serde_json::Map<_, _>>();
    Value::Object(entries)
}

fn file_values(fields: &FormFields) -> HelperResult {
    let data = convert_upload_files_to_file_data(fields)
        .map_err(|error| failure("AttributeError", error.to_string()))?;
    let values = data
        .into_iter()
        .map(|(name, value)| {
            let value = match value {
                FileDataValue::Unchanged(value) => form_value(&value),
                FileDataValue::Files(files) => Value::Array(
                    files
                        .into_iter()
                        .map(|file| {
                            json!([
                                file.filename,
                                base64::Engine::encode(
                                    &base64::engine::general_purpose::STANDARD,
                                    file.content
                                ),
                                file.content_type,
                            ])
                        })
                        .collect(),
                ),
            };
            (name, value)
        })
        .collect::<serde_json::Map<_, _>>();
    Ok(Value::Object(values))
}

fn form_value(value: &FormValue) -> Value {
    match value {
        FormValue::Text(value) => Value::String(value.clone()),
        FormValue::Number(value) => Value::Number(value.clone()),
        FormValue::NonFinite(value) => Value::String(value.token().to_owned()),
        FormValue::List(values) => Value::Array(values.iter().map(form_value).collect()),
        FormValue::Other(value) => serde_json::to_value(value).unwrap_or(Value::Null),
        FormValue::Upload(upload) => json!({
            "filename": upload.filename,
            "content_type": upload.content_type,
            "content": upload.content,
            "size": upload.size,
        }),
    }
}

fn failure(kind: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: kind.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
