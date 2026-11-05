use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_prompts::media_metadata::url_mime::{image_mime_type_from_url, infer_content_type};
use betterlite_prompts::media_metadata::{
    FileBody, FileData, extract_file_data, extract_file_metadata,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum WireBody {
    Bytes { content_base64: String },
    BareString { content: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum WireFileData {
    Direct {
        content: WireBody,
    },
    Tuple2 {
        filename: Option<String>,
        content: WireBody,
    },
    Tuple3 {
        filename: Option<String>,
        content: WireBody,
        content_type: Option<String>,
    },
    Tuple4 {
        filename: Option<String>,
        content: WireBody,
        content_type: Option<String>,
        headers: BTreeMap<String, String>,
    },
    InMemoryFile {
        name: String,
        content_type: String,
        content_base64: String,
    },
}

fn file_body(wire: WireBody) -> Result<FileBody, Box<HelperError>> {
    match wire {
        WireBody::Bytes { content_base64 } => BASE64
            .decode(content_base64)
            .map(FileBody::Bytes)
            .map_err(|error| input_error(error.to_string())),
        WireBody::BareString { content } => Ok(FileBody::BareString(content)),
    }
}

// Every shape maps onto FileData::Tuple4; the production parts_of() flattens all five the same way.
fn file_data(wire: WireFileData) -> Result<FileData, Box<HelperError>> {
    let (filename, content_type, headers, content) = match wire {
        WireFileData::Direct { content } => (None, None, BTreeMap::new(), file_body(content)?),
        WireFileData::Tuple2 { filename, content } => {
            (filename, None, BTreeMap::new(), file_body(content)?)
        }
        WireFileData::Tuple3 {
            filename,
            content,
            content_type,
        } => (filename, content_type, BTreeMap::new(), file_body(content)?),
        WireFileData::Tuple4 {
            filename,
            content,
            content_type,
            headers,
        } => (filename, content_type, headers, file_body(content)?),
        WireFileData::InMemoryFile {
            name,
            content_type,
            content_base64,
        } => {
            let bytes = BASE64
                .decode(content_base64)
                .map_err(|error| input_error(error.to_string()))?;
            (
                Some(name),
                Some(content_type),
                BTreeMap::new(),
                FileBody::Bytes(bytes),
            )
        }
    };
    Ok(FileData::Tuple4 {
        filename,
        content,
        content_type,
        headers,
    })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "extract_file_metadata")]
    Metadata { file_data: WireFileData },
    #[serde(rename = "extract_file_data")]
    Data { file_data: WireFileData },
    #[serde(rename = "image_mime_type_from_url")]
    ImageMimeFromUrl { url: String },
    #[serde(rename = "infer_content_type_from_url_and_content")]
    Infer {
        url: String,
        content_base64: String,
        current_content_type: Option<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Metadata { file_data: wire } => {
                let parsed = file_data(wire.clone())?;
                let (filename, content_type) = extract_file_metadata(&parsed);
                Ok(json!({"filename": filename, "content_type": content_type}))
            }
            Input::Data { file_data: wire } => {
                let parsed = file_data(wire.clone())?;
                let extracted =
                    extract_file_data(&parsed).map_err(|error| value_error(error.to_string()))?;
                Ok(json!({
                    "filename": extracted.filename,
                    "content_base64": BASE64.encode(extracted.content),
                    "content_type": extracted.content_type,
                    "headers": extracted.headers,
                }))
            }
            Input::ImageMimeFromUrl { url } => Ok(Value::from(image_mime_type_from_url(url))),
            Input::Infer {
                url,
                content_base64,
                current_content_type,
            } => {
                let content = BASE64
                    .decode(content_base64)
                    .map_err(|error| input_error(error.to_string()))?;
                let mime = infer_content_type(url, &content, current_content_type.as_deref())
                    .map_err(|error| value_error(error.to_string()))?;
                Ok(Value::String(mime))
            }
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

fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
