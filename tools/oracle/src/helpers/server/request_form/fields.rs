use std::collections::BTreeMap;

use betterlite_core::api::request_form::annotations::{
    FormAnnotation, FormAnnotations, Qualifier, numeric_form_fields,
};
use betterlite_core::api::request_form::coercion::coerce_numeric_form_fields;
use betterlite_core::api::request_form::convert::convert_uploads_to_file_data;
use betterlite_core::api::request_form::values::{
    FormFields, FormPart, FormParts, FormValue, PartValue, UploadFile, get_form_data,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::projection::{file_data_value, form_fields_value};
use super::{Input, PartInput, failure};

pub(super) fn form_data(input: &Input) -> HelperResult {
    let fields = get_form_data(&form_parts(input)?)
        .map_err(|error| failure("AttributeError", error.to_string()))?;
    let data = convert_uploads_to_file_data(&fields)
        .map_err(|error| failure("AttributeError", error.to_string()))?;
    file_data_value(&data)
}

pub(super) fn numeric(input: &Input) -> HelperResult {
    let fields = numeric_fields(input)?;
    let annotations = annotations_of(&input.annotations)?;
    let numeric = numeric_form_fields(&annotations);
    let coerced = coerce_numeric_form_fields(&fields, &numeric);
    form_fields_value(&coerced)
}

pub(super) fn direct_fields(input: &Input) -> Result<FormFields, Box<HelperError>> {
    let mut fields = FormFields::new();
    for part in &input.parts {
        let part = form_part(part)?;
        let value = match part.value {
            PartValue::Text(text) => FormValue::Text(text),
            PartValue::Upload(upload) => FormValue::Upload(upload),
        };
        fields.insert(part.name, value);
    }
    Ok(fields)
}

fn form_parts(input: &Input) -> Result<FormParts, Box<HelperError>> {
    let mut items = Vec::with_capacity(input.parts.len());
    for part in &input.parts {
        items.push(form_part(part)?);
    }
    Ok(FormParts { items })
}

fn form_part(part: &PartInput) -> Result<FormPart, Box<HelperError>> {
    let (name, value) = match part {
        PartInput::Text { name, value } => (name.clone(), PartValue::Text(value.clone())),
        PartInput::Upload {
            name,
            filename,
            content_type,
            content,
        } => (
            name.clone(),
            PartValue::Upload(upload_file(filename, content_type.as_deref(), content)?),
        ),
    };
    Ok(FormPart { name, value })
}

fn upload_file(
    filename: &str,
    content_type: Option<&str>,
    content: &str,
) -> Result<UploadFile, Box<HelperError>> {
    let bytes = content.as_bytes().to_vec();
    let size =
        u64::try_from(bytes.len()).map_err(|error| failure("OverflowError", error.to_string()))?;
    Ok(UploadFile {
        filename: filename.to_owned(),
        content_type: content_type.map(str::to_owned),
        content: bytes,
        size: Some(size),
    })
}

fn numeric_fields(input: &Input) -> Result<FormFields, Box<HelperError>> {
    let mut fields = FormFields::new();
    for (name, value) in &input.fields {
        fields.insert(name.clone(), numeric_value(value));
    }
    Ok(fields)
}

fn numeric_value(value: &Json) -> FormValue {
    match value {
        Json::Number(number) => FormValue::Number(number.clone()),
        Json::Text(text) => FormValue::Text(text.clone()),
        Json::List(items) => FormValue::List(items.iter().map(numeric_value).collect()),
        Json::Null | Json::Bool(_) | Json::Object(_) => FormValue::Other(value.clone()),
    }
}

fn annotations_of(kinds: &BTreeMap<String, String>) -> Result<FormAnnotations, Box<HelperError>> {
    let mut annotations = FormAnnotations::new();
    for (name, kind) in kinds {
        annotations.insert(name.clone(), annotation_of(kind)?);
    }
    Ok(annotations)
}

fn annotation_of(kind: &str) -> Result<FormAnnotation, Box<HelperError>> {
    let annotation = match kind {
        "int" => FormAnnotation::Int,
        "float" => FormAnnotation::Float,
        "optional_int" => {
            FormAnnotation::Union(vec![FormAnnotation::Int, FormAnnotation::NoneType])
        }
        "optional_float" => {
            FormAnnotation::Union(vec![FormAnnotation::Float, FormAnnotation::NoneType])
        }
        "annotated_int" => {
            FormAnnotation::Qualified(Qualifier::Annotated, Box::new(FormAnnotation::Int))
        }
        "annotated_optional_float" => FormAnnotation::Qualified(
            Qualifier::Annotated,
            Box::new(FormAnnotation::Union(vec![
                FormAnnotation::Float,
                FormAnnotation::NoneType,
            ])),
        ),
        "required_int" => {
            FormAnnotation::Qualified(Qualifier::Required, Box::new(FormAnnotation::Int))
        }
        "notrequired_int" => {
            FormAnnotation::Qualified(Qualifier::NotRequired, Box::new(FormAnnotation::Int))
        }
        "readonly_int" => {
            FormAnnotation::Qualified(Qualifier::ReadOnly, Box::new(FormAnnotation::Int))
        }
        "bool" | "str" | "literal" | "list" | "union" | "other" => FormAnnotation::Other,
        _other => return Err(failure("KeyError", kind.to_owned())),
    };
    Ok(annotation)
}
