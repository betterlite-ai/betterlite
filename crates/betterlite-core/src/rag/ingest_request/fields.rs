use crate::api::request_body::{JsonObject, ParsedJson, parse_json_text};
use crate::api::request_form::values::{FormFields, FormValue, UploadFile, get_form_data};

use super::IngestBody;
use super::base64;
use super::error::IngestRequestError;

pub(super) struct IngestFields {
    pub(super) ingest_options: JsonObject,
    pub(super) file: Option<UploadFile>,
    pub(super) file_url: Option<String>,
    pub(super) file_id: Option<String>,
}

pub(super) fn multipart(body: IngestBody<'_>) -> Result<IngestFields, IngestRequestError> {
    let IngestBody::Multipart(parts) = body else {
        return Err(IngestRequestError::BodyKindMismatch);
    };
    let form = get_form_data(parts)?;
    let file = match form.get("file") {
        Some(FormValue::Upload(upload)) => Some(upload.clone()),
        Some(
            FormValue::Text(_)
            | FormValue::List(_)
            | FormValue::Number(_)
            | FormValue::Other(_)
            | FormValue::NonFinite(_),
        )
        | None => None,
    };
    let request = request_field(&form)?.unwrap_or_default();
    fields_from(&request, file)
}

pub(super) fn json(body: IngestBody<'_>) -> Result<IngestFields, IngestRequestError> {
    let IngestBody::Json(data) = body else {
        return Err(IngestRequestError::BodyKindMismatch);
    };
    let file = file_field(data)?;
    fields_from(data, file)
}

fn fields_from(
    body: &JsonObject,
    file: Option<UploadFile>,
) -> Result<IngestFields, IngestRequestError> {
    Ok(IngestFields {
        ingest_options: object_field(body, "ingest_options")?.unwrap_or_default(),
        file,
        file_url: text_field(body, "file_url", "file_url")?,
        file_id: text_field(body, "file_id", "file_id")?,
    })
}

fn request_field(form: &FormFields) -> Result<Option<JsonObject>, IngestRequestError> {
    match form.get("request") {
        None => Ok(None),
        Some(FormValue::Text(text)) => match parse_json_text(text) {
            ParsedJson::Object(object) => Ok(Some(object)),
            ParsedJson::NotObject => Err(IngestRequestError::RequestNotObject),
            ParsedJson::Invalid => Err(IngestRequestError::RequestInvalidJson),
        },
        Some(
            FormValue::Upload(_)
            | FormValue::List(_)
            | FormValue::Number(_)
            | FormValue::Other(_)
            | FormValue::NonFinite(_),
        ) => Err(IngestRequestError::RequestNotText),
    }
}

fn file_field(body: &JsonObject) -> Result<Option<UploadFile>, IngestRequestError> {
    let Some(fields) = object_field(body, "file")? else {
        return Ok(None);
    };
    let Some(filename) = text_field(&fields, "filename", "file.filename")? else {
        return Ok(None);
    };
    let Some(content) = text_field(&fields, "content", "file.content")? else {
        return Ok(None);
    };
    Ok(Some(UploadFile {
        filename,
        content_type: None,
        content: base64::decode(&content)
            .map_err(|source| IngestRequestError::InvalidBase64 { source })?,
        size: None,
    }))
}

fn object_field(
    body: &JsonObject,
    field: &'static str,
) -> Result<Option<JsonObject>, IngestRequestError> {
    match body.get(field) {
        None => Ok(None),
        Some(value) if value.is_null() => Ok(None),
        Some(value) => {
            let Some(object) = value.as_object() else {
                return Err(IngestRequestError::NotAnObject { field });
            };
            Ok(Some(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ))
        }
    }
}

fn text_field(
    body: &JsonObject,
    key: &str,
    field: &'static str,
) -> Result<Option<String>, IngestRequestError> {
    let Some(value) = body.get(key).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|text| Some(text.to_owned()))
        .ok_or(IngestRequestError::NotText { field })
}
