use betterlite_core::api::api_boundaries::{
    ApiBoundariesInput, ApiLimits, ItemsBoundary, JsonBoundary,
};
use betterlite_core::api::request_body::{
    DecodedJson, MediaTypeKind, RequestBodyError, classify_media_type,
};
use betterlite_core::api::request_form::values::{
    FormFields, FormPart, FormParts, PartValue, UploadFile, get_form_data,
};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RequestFormError {
    #[error("request form body is too large")]
    BodyTooLarge,
    #[error("request form has too many parts")]
    TooManyParts,
    #[error("invalid request form content type")]
    ContentType,
    #[error("invalid request form body")]
    InvalidBody,
    #[error("request form field name is missing")]
    MissingName,
    #[error("Unsupported content type: {0}")]
    UnsupportedContentType(String),
    #[error(transparent)]
    RequestBody(#[from] RequestBodyError),
}

#[derive(Debug, PartialEq, Eq)]
pub enum RequestFormBody {
    Empty,
    Json(DecodedJson),
    Form(FormFields),
}

pub fn get_request_body(
    method: &str,
    content_type: &str,
    body: &[u8],
    limits: &ApiLimits,
) -> Result<RequestFormBody, RequestFormError> {
    if method != "POST" {
        return Ok(RequestFormBody::Empty);
    }
    match classify_media_type(content_type) {
        MediaTypeKind::Json => Ok(RequestFormBody::Json(
            super::request_body::decode_request_body(body, limits)?,
        )),
        MediaTypeKind::Form => {
            let parts = decode_request_form(body, content_type.as_bytes(), limits)?;
            Ok(RequestFormBody::Form(
                get_form_data(&parts).map_err(|_error| RequestFormError::InvalidBody)?,
            ))
        }
        MediaTypeKind::Binary | MediaTypeKind::Other => Err(
            RequestFormError::UnsupportedContentType(content_type.to_owned()),
        ),
    }
}

// decode_request_form: URL encoded and multipart bytes decode into ordered parts before any field policy runs
pub fn decode_request_form(
    bytes: &[u8],
    content_type: &[u8],
    limits: &ApiLimits,
) -> Result<FormParts, RequestFormError> {
    limits
        .check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: bytes.len(),
        }))
        .map_err(|_error| RequestFormError::BodyTooLarge)?;
    limits
        .check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: content_type.len(),
        }))
        .map_err(|_error| RequestFormError::BodyTooLarge)?;
    let content_type =
        std::str::from_utf8(content_type).map_err(|_error| RequestFormError::ContentType)?;
    let media_type = content_type.split(';').next().unwrap_or(content_type);
    match media_type.trim().to_ascii_lowercase().as_str() {
        "application/x-www-form-urlencoded" => decode_urlencoded(bytes, limits),
        "multipart/form-data" => decode_multipart(bytes, content_type, limits),
        _other => Err(RequestFormError::ContentType),
    }
}

fn decode_urlencoded(bytes: &[u8], limits: &ApiLimits) -> Result<FormParts, RequestFormError> {
    let part_count = if bytes.is_empty() {
        0
    } else {
        bytes
            .iter()
            .filter(|byte| **byte == b'&')
            .count()
            .saturating_add(1)
    };
    check_part_count(part_count, limits)?;
    let mut items = Vec::with_capacity(part_count);
    for (name, value) in form_urlencoded::parse(bytes) {
        items.push(FormPart {
            name: name.into_owned(),
            value: PartValue::Text(value.into_owned()),
        });
    }
    Ok(FormParts { items })
}

fn decode_multipart(
    bytes: &[u8],
    content_type: &str,
    limits: &ApiLimits,
) -> Result<FormParts, RequestFormError> {
    use multipart::disposition::disposition_param_bytes;
    let boundary_bytes = disposition_param_bytes(content_type.as_bytes(), "boundary")
        .filter(|value| !value.is_empty())
        .ok_or(RequestFormError::ContentType)?;
    let boundary =
        std::str::from_utf8(&boundary_bytes).map_err(|_error| RequestFormError::ContentType)?;
    let charset = disposition_param_bytes(content_type.as_bytes(), "charset").map_or_else(
        || "utf-8".to_owned(),
        |value| multipart::charset::decode_part(&value, "latin1"),
    );
    if bytes.is_empty() {
        return Ok(FormParts { items: Vec::new() });
    }
    let mut items = Vec::new();
    for item in multipart::raw_split::split_items(bytes, boundary) {
        let item = item.map_err(|_error| RequestFormError::InvalidBody)?;
        check_part_count(items.len().saturating_add(1), limits)?;
        items.push(read_part(&item, &charset)?);
    }
    Ok(FormParts { items })
}

fn read_part(
    item: &multipart::raw_split::RawItem<'_>,
    charset: &str,
) -> Result<FormPart, RequestFormError> {
    use multipart::charset::decode_part;
    use multipart::disposition::disposition_param_bytes;
    let disposition = item
        .content_disposition
        .ok_or(RequestFormError::InvalidBody)?;
    let name_bytes =
        disposition_param_bytes(disposition, "name").ok_or(RequestFormError::MissingName)?;
    let name = decode_part(&name_bytes, charset);
    let value = match disposition_param_bytes(disposition, "filename") {
        Some(filename) => PartValue::Upload(UploadFile {
            filename: decode_part(&filename, charset),
            content_type: item.content_type.map(|value| decode_part(value, "latin1")),
            content: item.data.to_vec(),
            size: Some(
                u64::try_from(item.data.len()).map_err(|_error| RequestFormError::BodyTooLarge)?,
            ),
        }),
        None => PartValue::Text(decode_part(item.data, charset)),
    };
    Ok(FormPart { name, value })
}

fn check_part_count(count: usize, limits: &ApiLimits) -> Result<(), RequestFormError> {
    limits
        .check(ApiBoundariesInput::Items(ItemsBoundary {
            known_count: count,
        }))
        .map(|_result| ())
        .map_err(|_error| RequestFormError::TooManyParts)
}

pub(crate) mod multipart;

mod tests;
