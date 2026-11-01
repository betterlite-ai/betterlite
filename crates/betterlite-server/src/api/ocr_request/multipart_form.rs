use std::io::Error as IoError;

use betterlite_ocr::json::{Json, Object};
use futures_util::StreamExt as _;
use multipart_rs::{MultipartItem, MultipartReader};

use super::error::OcrRequestError;
use super::upload::{Upload, read_upload};
use super::{FormField, FormValue, OcrDocument, ParsedForm};

const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;

pub(super) async fn parse_body(
    body: &[u8],
    content_type: &str,
) -> Result<ParsedForm, OcrRequestError> {
    let headers = [("content-type".to_owned(), content_type.to_owned())];
    let mut reader = MultipartReader::<IoError>::from_data_with_headers(body, &headers)
        .map_err(OcrRequestError::MultipartParse)?;
    let mut form = ParsedForm::new();
    while let Some(item) = reader.next().await {
        form.push(read_item(&item.map_err(OcrRequestError::MultipartParse)?)?);
    }
    Ok(form)
}

fn disposition_param(disposition: &str, key: &str) -> Option<String> {
    disposition.split(';').map(str::trim).find_map(|part| {
        let (part_key, part_value) = part.split_once('=')?;
        (part_key == key).then(|| part_value.trim_matches('"').to_owned())
    })
}

fn read_item(item: &MultipartItem) -> Result<FormField, OcrRequestError> {
    let header = |key: &str| {
        item.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    };
    let disposition = header("content-disposition").unwrap_or_default();
    let name = disposition_param(disposition, "name").unwrap_or_default();
    let filename = disposition_param(disposition, "filename");
    let content_type = header("content-type").map(ToOwned::to_owned);
    let bytes = item.data.to_vec();
    let value = if filename.is_some() {
        FormValue::File {
            filename,
            content_type,
            bytes,
        }
    } else {
        FormValue::Text(String::from_utf8(bytes).map_err(OcrRequestError::InvalidTextEncoding)?)
    };
    Ok(FormField { name, value })
}

// The constructed upload document replaces a submitted "document" field, which is skipped.
pub(super) fn build(form: &ParsedForm) -> Result<(Option<OcrDocument>, Object), OcrRequestError> {
    let file_field = form
        .iter()
        .find(|field| field.name == "file")
        .ok_or(OcrRequestError::MissingFile)?;
    let FormValue::File {
        content_type,
        bytes,
        ..
    } = &file_field.value
    else {
        return Err(OcrRequestError::MissingFile);
    };
    let mut upload = Upload {
        bytes: bytes.clone(),
        position: 0,
    };
    let file_content = read_upload(&mut upload, MAX_FILE_BYTES)?;
    let mime_type = supplied_mime_type(content_type.as_deref());
    let document = OcrDocument::Upload {
        bytes: file_content,
        mime_type,
    };
    let mut fields = Object::new();
    for field in form {
        if field.name == "file" || field.name == "document" {
            continue;
        }
        if let FormValue::Text(text) = &field.value {
            // Tries JSON first (e.g. pages=[0,1,2]); keeps plain text when it does not decode.
            let value = serde_json::from_str(text).unwrap_or_else(|_| Json::Text(text.clone()));
            fields.insert(field.name.clone(), value);
        }
    }
    Ok((Some(document), fields))
}

fn supplied_mime_type(content_type: Option<&str>) -> Option<String> {
    let base = content_type?.split(';').next()?.trim();
    if base.is_empty() || base == "application/octet-stream" {
        return None;
    }
    Some(base.to_owned())
}

mod tests;
