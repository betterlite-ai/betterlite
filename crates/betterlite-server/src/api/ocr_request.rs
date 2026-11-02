pub mod error;
mod format;
mod json_body;
mod multipart_form;
pub mod native_response;
mod upload;

use betterlite_ocr::json::Object;
use betterlite_ocr::request_format::RequestFormat;

use error::OcrRequestError;

#[derive(Debug, Clone, PartialEq)]
pub enum OcrDocument {
    Upload {
        bytes: Vec<u8>,
        filename: Option<String>,
        mime_type: Option<String>,
    },
    Inline(Object),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormValue {
    Text(String),
    File {
        filename: Option<String>,
        content_type: Option<String>,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    pub name: String,
    pub value: FormValue,
}

pub type ParsedForm = Vec<FormField>;

pub struct RawOcrRequest {
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub header_req_format: Option<String>,
    pub cached_form: Option<ParsedForm>,
}

#[derive(Debug)]
pub struct ParsedOcrRequest {
    pub document: Option<OcrDocument>,
    pub fields: Object,
    pub req_format: Option<RequestFormat>,
}

pub fn parse_ocr_request(raw: &RawOcrRequest) -> Result<ParsedOcrRequest, OcrRequestError> {
    let (document, parsed_fields) = parse_ocr_request_body(raw)?;
    let (fields, req_format) = format::resolve(parsed_fields, raw.header_req_format.as_deref())?;
    Ok(ParsedOcrRequest {
        document,
        fields,
        req_format,
    })
}

fn parse_ocr_request_body(
    raw: &RawOcrRequest,
) -> Result<(Option<OcrDocument>, Object), OcrRequestError> {
    let is_multipart = raw.content_type.as_deref().is_some_and(|content_type| {
        content_type
            .to_ascii_lowercase()
            .contains("multipart/form-data")
    });
    if is_multipart {
        let content_type = raw.content_type.as_deref().unwrap_or_default();
        let form = multipart_form::parse_body(&raw.body, content_type)?;
        return multipart_form::build(&form);
    }
    if raw.body.is_empty() {
        return raw
            .cached_form
            .as_ref()
            .map_or_else(|| Err(OcrRequestError::EmptyBody), multipart_form::build);
    }
    let parsed = json_body::parse(&raw.body)?;
    Ok((parsed.document, parsed.fields))
}

mod tests;
