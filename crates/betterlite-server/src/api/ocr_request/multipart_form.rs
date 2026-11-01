use std::convert::Infallible;

use betterlite_ocr::json::{Json, Object};
use bytes::Bytes;
use futures_util::stream;
use multer::Multipart;

use super::error::OcrRequestError;
use super::upload::{Upload, read_upload};
use super::{FormField, FormValue, OcrDocument, ParsedForm};

const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;

pub(super) async fn parse_body(
    body: &[u8],
    content_type: &str,
) -> Result<ParsedForm, OcrRequestError> {
    let boundary = multer::parse_boundary(content_type).map_err(OcrRequestError::MultipartParse)?;
    let owned_body = Bytes::copy_from_slice(body);
    let body_stream = stream::once(async move { Ok::<Bytes, Infallible>(owned_body) });
    let mut multipart = Multipart::new(body_stream, boundary);
    let mut form = ParsedForm::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(OcrRequestError::MultipartParse)?
    {
        form.push(read_field(field).await?);
    }
    Ok(form)
}

async fn read_field(field: multer::Field<'static>) -> Result<FormField, OcrRequestError> {
    let name = field.name().unwrap_or_default().to_owned();
    let filename = field.file_name().map(str::to_owned);
    let content_type = field.content_type().map(ToString::to_string);
    let bytes = field
        .bytes()
        .await
        .map_err(OcrRequestError::MultipartParse)?;
    let value = if filename.is_some() || content_type.is_some() {
        FormValue::File {
            filename,
            content_type,
            bytes: bytes.to_vec(),
        }
    } else {
        FormValue::Text(
            String::from_utf8(bytes.to_vec()).map_err(OcrRequestError::InvalidTextEncoding)?,
        )
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
