use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_ocr::request_format::RequestFormat;
use betterlite_server::api::ocr_request::error::OcrRequestError;
use betterlite_server::api::ocr_request::{
    FormField, FormValue, OcrDocument, ParsedForm, ParsedOcrRequest, RawOcrRequest,
    parse_ocr_request,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

const BOUNDARY: &str = "OracleB3300Boundary";

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    json_body: Option<Value>,
    #[serde(default)]
    multipart: Option<MultipartInput>,
    #[serde(default)]
    use_cached_form: bool,
    #[serde(default)]
    header_req_format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MultipartInput {
    #[serde(default)]
    file_base64: Option<String>,
    #[serde(default)]
    file_filler_bytes: Option<usize>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    file_content_type: Option<String>,
    #[serde(default)]
    fields: Vec<(String, String)>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let raw = build_raw_request(input)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| input_error(error.to_string()))?;
        let parsed = runtime
            .block_on(parse_ocr_request(&raw))
            .map_err(|error| parse_error(&error))?;
        Ok(normalize(parsed))
    }
}

fn build_raw_request(input: &Input) -> Result<RawOcrRequest, Box<HelperError>> {
    if let Some(multipart) = &input.multipart {
        let content_type = Some(format!("multipart/form-data; boundary={BOUNDARY}"));
        if input.use_cached_form {
            let cached_form = build_cached_form(multipart)?;
            return Ok(RawOcrRequest {
                content_type,
                body: Vec::new(),
                header_req_format: input.header_req_format.clone(),
                cached_form: Some(cached_form),
            });
        }
        let body = build_multipart_body(multipart)?;
        return Ok(RawOcrRequest {
            content_type,
            body,
            header_req_format: input.header_req_format.clone(),
            cached_form: None,
        });
    }
    let body = match &input.json_body {
        Some(value) => serde_json::to_vec(value).map_err(|error| input_error(error.to_string()))?,
        None => Vec::new(),
    };
    Ok(RawOcrRequest {
        content_type: Some("application/json".to_owned()),
        body,
        header_req_format: input.header_req_format.clone(),
        cached_form: None,
    })
}

fn file_bytes(multipart: &MultipartInput) -> Result<Vec<u8>, Box<HelperError>> {
    if let Some(filler) = multipart.file_filler_bytes {
        return Ok(vec![b'A'; filler]);
    }
    multipart
        .file_base64
        .as_ref()
        .map(|encoded| BASE64.decode(encoded))
        .transpose()
        .map(Option::unwrap_or_default)
        .map_err(|error| input_error(error.to_string()))
}

fn build_cached_form(multipart: &MultipartInput) -> Result<ParsedForm, Box<HelperError>> {
    let bytes = file_bytes(multipart)?;
    let mut form = vec![FormField {
        name: "file".to_owned(),
        value: FormValue::File {
            filename: multipart.file_name.clone(),
            content_type: multipart.file_content_type.clone(),
            bytes,
        },
    }];
    for (name, value) in &multipart.fields {
        form.push(FormField {
            name: name.clone(),
            value: FormValue::Text(value.clone()),
        });
    }
    Ok(form)
}

fn build_multipart_body(multipart: &MultipartInput) -> Result<Vec<u8>, Box<HelperError>> {
    let bytes = file_bytes(multipart)?;
    let filename = multipart.file_name.as_deref().unwrap_or("upload.bin");
    let content_type_header = multipart
        .file_content_type
        .as_deref()
        .map_or_else(String::new, |value| format!("Content-Type: {value}\r\n"));
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n{content_type_header}\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&bytes);
    body.extend_from_slice(b"\r\n");
    for (name, value) in &multipart.fields {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    Ok(body)
}

fn normalize(parsed: ParsedOcrRequest) -> Value {
    let document = parsed.document.map_or(Value::Null, document_value);
    let fields = serde_json::to_value(&parsed.fields).unwrap_or(Value::Null);
    let req_format = parsed.req_format.map(wire_format);
    json!({"document": document, "fields": fields, "req_format": req_format})
}

const fn wire_format(format: RequestFormat) -> &'static str {
    match format {
        RequestFormat::Litellm => "litellm",
        RequestFormat::Native => "native",
    }
}

fn document_value(document: OcrDocument) -> Value {
    match document {
        OcrDocument::Upload { bytes, mime_type } => json!({
            "kind": "upload",
            "bytes_base64": BASE64.encode(bytes),
            "mime_type": mime_type,
        }),
        OcrDocument::Inline(object) => {
            let mut value = serde_json::to_value(&object).unwrap_or(Value::Null);
            if let Value::Object(map) = &mut value {
                map.insert("kind".to_owned(), Value::String("inline".to_owned()));
            }
            value
        }
    }
}

fn input_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn parse_error(error: &OcrRequestError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OcrRequestError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
