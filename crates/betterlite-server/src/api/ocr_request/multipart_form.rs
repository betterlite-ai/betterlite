use std::io::Error as IoError;

use betterlite_ocr::json::{Json, Object};
use encoding_rs::Encoding;
use futures_util::StreamExt as _;
use multipart_rs::{MultipartError, MultipartItem, MultipartReader};

use super::error::OcrRequestError;
use super::upload::read_upload;
use super::{FormField, FormValue, OcrDocument, ParsedForm};

const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
const MAX_FIELDS: usize = 1000;
const MAX_FILES: usize = 1000;
const MAX_PART_BYTES: usize = 1024 * 1024;

pub(super) async fn parse_body(
    body: &[u8],
    content_type: &str,
) -> Result<ParsedForm, OcrRequestError> {
    let headers = [("content-type".to_owned(), content_type.to_owned())];
    let mut reader = MultipartReader::<IoError>::from_data_with_headers(body, &headers)
        .map_err(OcrRequestError::MultipartParse)?;
    let charset = charset_param(content_type);
    let mut form = ParsedForm::new();
    let mut fields = 0_usize;
    let mut files = 0_usize;
    while let Some(item) = reader.next().await {
        let field = read_item(&item.map_err(OcrRequestError::MultipartParse)?, &charset)?;
        match &field.value {
            FormValue::File { .. } => files = files.saturating_add(1),
            FormValue::Text(_) => fields = fields.saturating_add(1),
        }
        if files > MAX_FILES || fields > MAX_FIELDS {
            return Err(OcrRequestError::MultipartParse(
                MultipartError::InvalidItemHeader,
            ));
        }
        form.push(field);
    }
    Ok(form)
}

// Mirrors Starlette's MultiPartParser: the charset comes from the request's own Content-Type, never a part's.
fn charset_param(content_type: &str) -> String {
    content_type
        .split(';')
        .skip(1)
        .find_map(|part| {
            let trimmed = part.trim();
            let (key, value) = trimmed.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| value.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| "utf-8".to_owned())
}

// _user_safe_decode: decode with the charset, falling back to latin-1 (which never fails) on an unrecognized charset or a decode error.
fn decode_part(bytes: &[u8], charset: &str) -> String {
    Encoding::for_label(charset.as_bytes()).map_or_else(
        || bytes.iter().copied().map(char::from).collect(),
        |encoding| {
            let (decoded, had_errors) = encoding.decode_without_bom_handling(bytes);
            if had_errors {
                bytes.iter().copied().map(char::from).collect()
            } else {
                decoded.into_owned()
            }
        },
    )
}

fn disposition_param(disposition: &str, key: &str) -> Option<String> {
    disposition.split(';').map(str::trim).find_map(|part| {
        let (part_key, part_value) = part.split_once('=')?;
        (part_key == key).then(|| part_value.trim_matches('"').to_owned())
    })
}

fn read_item(item: &MultipartItem, charset: &str) -> Result<FormField, OcrRequestError> {
    let header = |key: &str| {
        item.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    };
    let disposition = header("content-disposition").unwrap_or_default();
    let name = disposition_param(disposition, "name").ok_or(OcrRequestError::MultipartParse(
        MultipartError::InvalidItemHeader,
    ))?;
    let filename = disposition_param(disposition, "filename");
    let content_type = header("content-type").map(ToOwned::to_owned);
    if filename.is_some() {
        return Ok(FormField {
            name,
            value: FormValue::File {
                filename,
                content_type,
                bytes: item.data.to_vec(),
            },
        });
    }
    if item.data.len() > MAX_PART_BYTES {
        return Err(OcrRequestError::MultipartParse(
            MultipartError::InvalidItemHeader,
        ));
    }
    Ok(FormField {
        name,
        value: FormValue::Text(decode_part(&item.data, charset)),
    })
}

// The constructed upload document replaces a submitted "document" field, which is skipped.
pub(super) fn build(form: &ParsedForm) -> Result<(Option<OcrDocument>, Object), OcrRequestError> {
    let file_field = form
        .iter()
        .rfind(|field| field.name == "file")
        .ok_or(OcrRequestError::MissingFile)?;
    let FormValue::File {
        filename,
        content_type,
        bytes,
    } = &file_field.value
    else {
        return Err(OcrRequestError::MissingFile);
    };
    let file_content = read_upload(bytes, MAX_FILE_BYTES)?;
    let mime_type = supplied_mime_type(content_type.as_deref());
    let document = OcrDocument::Upload {
        bytes: file_content,
        filename: filename.clone(),
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
