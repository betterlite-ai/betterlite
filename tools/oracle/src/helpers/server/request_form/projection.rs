use betterlite_core::api::request_form::convert::{FileData, FileDataValue, FileTuple};
use betterlite_core::api::request_form::values::{FormFields, FormValue, UploadFile};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Map, Number, Value};
use std::str;

pub(super) fn file_data_value(data: &FileData) -> HelperResult {
    let mut entries = Map::new();
    for (name, value) in data {
        let projected = match value {
            FileDataValue::Unchanged(unchanged) => form_value(unchanged)?,
            FileDataValue::Files(files) => Value::Array(
                files
                    .iter()
                    .map(file_tuple_value)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        };
        entries.insert(name.clone(), projected);
    }
    Ok(Value::Object(entries))
}

pub(super) fn form_fields_value(fields: &FormFields) -> HelperResult {
    let mut entries = Map::new();
    for (name, value) in fields {
        entries.insert(name.clone(), form_value(value)?);
    }
    Ok(Value::Object(entries))
}

pub(super) fn form_value(value: &FormValue) -> HelperResult {
    Ok(match value {
        FormValue::Text(text) => Value::String(text.clone()),
        FormValue::Number(number) => Value::Number(number.clone()),
        FormValue::NonFinite(non_finite) => Value::String(non_finite.token().to_owned()),
        FormValue::List(items) => Value::Array(
            items
                .iter()
                .map(form_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        FormValue::Upload(upload) => upload_value(upload)?,
        FormValue::Other(json) => serde_json::to_value(json).or_value_error()?,
    })
}

fn file_tuple_value(file: &FileTuple) -> HelperResult {
    Ok(Value::Array(vec![
        Value::String(file.filename.clone()),
        Value::String(utf8(&file.content)?),
        file.content_type.clone().map_or(Value::Null, Value::String),
    ]))
}

fn upload_value(upload: &UploadFile) -> HelperResult {
    let mut entry = Map::new();
    entry.insert(
        "filename".to_owned(),
        Value::String(upload.filename.clone()),
    );
    entry.insert("content".to_owned(), Value::String(utf8(&upload.content)?));
    entry.insert(
        "content_type".to_owned(),
        upload
            .content_type
            .clone()
            .map_or(Value::Null, Value::String),
    );
    entry.insert(
        "size".to_owned(),
        upload
            .size
            .map_or(Value::Null, |size| Value::Number(Number::from(size))),
    );
    Ok(Value::Object(entry))
}

fn utf8(bytes: &[u8]) -> Result<String, Box<HelperError>> {
    str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| super::failure("UnicodeDecodeError", error.to_string()))
}
