use std::io::Error as IoError;
use std::str;

use betterlite_ocr::json::{Json, Object};
use multipart_rs::{MultipartError, MultipartReader};

use super::error::OcrRequestError;
use super::upload::read_upload;
use super::{FormField, FormValue, OcrDocument, ParsedForm};
use charset::decode_part;
use disposition::disposition_param_bytes;
use raw_split::{RawItem, split_items};

mod charset;
mod disposition;
mod raw_split;
mod tests;

const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
const MAX_FIELDS: usize = 1000;
const MAX_FILES: usize = 1000;
const MAX_PART_BYTES: usize = 1024 * 1024;

// multipart-rs's own reader rejects a header line that is not valid UTF-8 before it ever builds an item, so it is reused only to read the boundary and validate the Content-Type header; the body is then split by hand in raw_split, keeping headers as bytes until decode_part runs.
pub(super) fn parse_body(body: &[u8], content_type: &str) -> Result<ParsedForm, OcrRequestError> {
    let headers = [("content-type".to_owned(), content_type.to_owned())];
    let boundary = MultipartReader::<IoError>::from_data_with_headers(body, &headers)
        .map_err(OcrRequestError::MultipartParse)?
        .boundary;
    let charset = charset_param(content_type);
    let mut form = ParsedForm::new();
    let mut fields = 0_usize;
    let mut files = 0_usize;
    for item in split_items(body, &boundary)? {
        let field = read_item(&item, &charset)?;
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

fn read_item(item: &RawItem<'_>, charset: &str) -> Result<FormField, OcrRequestError> {
    let disposition = item.content_disposition.unwrap_or_default();
    let name_bytes = disposition_param_bytes(disposition, "name").ok_or(
        OcrRequestError::MultipartParse(MultipartError::InvalidItemHeader),
    )?;
    let name = decode_part(&name_bytes, charset);
    let filename =
        disposition_param_bytes(disposition, "filename").map(|bytes| decode_part(&bytes, charset));
    let content_type = item
        .content_type
        .and_then(|bytes| str::from_utf8(bytes).ok())
        .map(ToOwned::to_owned);
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
        value: FormValue::Text(decode_part(item.data, charset)),
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
