use std::collections::BTreeMap;

use betterlite_core::api::request_body::{JsonObject, ParsedJson, parse_json_text};
use betterlite_core::api::request_form::values::{FormPart, FormParts, PartValue, UploadFile};
use betterlite_core::rag::ingest_request::error::IngestRequestError;
use betterlite_core::rag::ingest_request::{
    IngestBody, IngestRequest, ParsedIngestRequest, SecuredDocument, parse_rag_ingest_request,
};
use betterlite_core::rag::upload_security::{EicarTestMalwareScanner, MAX_UPLOAD_SIZE_BYTES};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::{Deserialize, Serialize};

const ID: uuid::Uuid = uuid::Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000);

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Multipart { parts: Vec<Part> },
    Json { body: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Part {
    Text {
        name: String,
        text: String,
    },
    Upload {
        name: String,
        filename: String,
        content_hex: String,
        content_type: String,
    },
}

#[derive(Serialize)]
struct FileOutput {
    filename: String,
    content_hex: String,
    content_type: String,
}

#[derive(Serialize)]
struct Output {
    ingest_options: JsonObject,
    file: Option<FileOutput>,
    file_url: Option<String>,
    file_id: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Multipart { parts } => multipart_answer(parts),
            Input::Json { body } => json_answer(body),
        }
    }
}

fn multipart_answer(parts: &[Part]) -> HelperResult {
    let form = form_parts(parts)?;
    let mut headers = BTreeMap::new();
    headers.insert(
        "content-type".to_owned(),
        "multipart/form-data; boundary=betterlite".to_owned(),
    );
    answer(parse(&headers, IngestBody::Multipart(&form)))
}

fn json_answer(body: &str) -> HelperResult {
    let ParsedJson::Object(data) = parse_json_text(body) else {
        return Err(invalid_input("the JSON body must be a JSON object"));
    };
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    answer(parse(&headers, IngestBody::Json(&data)))
}

fn parse(
    headers: &BTreeMap<String, String>,
    body: IngestBody<'_>,
) -> Result<ParsedIngestRequest, IngestRequestError> {
    let request = IngestRequest { headers, body };
    parse_rag_ingest_request(
        &request,
        MAX_UPLOAD_SIZE_BYTES,
        &EicarTestMalwareScanner,
        || ID,
    )
}

fn form_parts(parts: &[Part]) -> Result<FormParts, Box<HelperError>> {
    let mut items = Vec::with_capacity(parts.len());
    for part in parts {
        items.push(form_part(part)?);
    }
    Ok(FormParts { items })
}

fn form_part(part: &Part) -> Result<FormPart, Box<HelperError>> {
    match part {
        Part::Text { name, text } => Ok(FormPart {
            name: name.clone(),
            value: PartValue::Text(text.clone()),
        }),
        Part::Upload {
            name,
            filename,
            content_hex,
            content_type,
        } => Ok(FormPart {
            name: name.clone(),
            value: PartValue::Upload(UploadFile {
                filename: filename.clone(),
                content_type: Some(content_type.clone()),
                content: hex::decode(content_hex)
                    .map_err(|error| invalid_input(&error.to_string()))?,
                size: None,
            }),
        }),
    }
}

fn answer(result: Result<ParsedIngestRequest, IngestRequestError>) -> HelperResult {
    let parsed = result.map_err(|error| http_error(&error))?;
    serde_json::to_value(Output {
        ingest_options: parsed.ingest_options,
        file: parsed.file.map(file_output),
        file_url: parsed.file_url,
        file_id: parsed.file_id,
    })
    .map_err(|error| invalid_input(&error.to_string()))
}

fn file_output(file: SecuredDocument) -> FileOutput {
    FileOutput {
        filename: file.safe_filename,
        content_hex: hex::encode(file.content),
        content_type: file.content_type,
    }
}

fn http_error(error: &IngestRequestError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "HTTPException".to_owned(),
        status: Some(IngestRequestError::STATUS),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

fn invalid_input(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
