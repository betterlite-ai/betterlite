use std::sync::{Arc, Mutex};

use betterlite_core::api::file_contracts::{is_batch_jsonl_file, valid_content_type};
use betterlite_openai::api::file_type_records::ByteStream;
use betterlite_openai::files::records::{CreatePurpose, FileCreate, FileUpload};
use betterlite_provider::api::file_contracts::config::{
    BaseFilesConfig as _, FileLogging, FileParams, FileResponse, FileUrlRequest, Headers,
    StreamContext,
};
use betterlite_provider::api::file_contracts::upload_stream::BaseFileUploadStream;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use minimal_config::{CompleteUrl, Config};

mod minimal_config;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    purpose: Option<String>,
    content_type: Option<String>,
    content: Option<String>,
    file: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaseDefaultsInput {
    response_body: String,
    optional_params: FileParams,
    litellm_params: FileParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileData {
    filename: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileUrlInput {
    api_base: String,
    api_key: String,
    model: String,
    optional_params: FileParams,
    litellm_params: FileParams,
    data: FileData,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamPassthroughInput {
    chunks_hex: Vec<String>,
    headers: Headers,
    request_url: String,
    litellm_params: FileParams,
    upload_chunks_hex: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    BatchExtractedPredicate { cases: Vec<Case> },
    BatchMetadataPredicate { cases: Vec<Case> },
    BaseDefaults(BaseDefaultsInput),
    FileUrlForwarding(FileUrlInput),
    StreamPassthroughAndReplay(StreamPassthroughInput),
}

struct Upload {
    chunks: Vec<Vec<u8>>,
}

impl BaseFileUploadStream for Upload {
    fn iter_bytes(&self) -> ByteStream {
        Box::new(self.chunks.clone().into_iter())
    }
}

#[derive(Debug, Clone, Copy)]
enum Predicate {
    Content,
    File,
}

fn predicate_results(cases: &[Case], predicate: Predicate) -> Value {
    let results: Vec<bool> = cases
        .iter()
        .map(|case| match predicate {
            Predicate::Content => is_batch_jsonl_file(
                case.purpose.as_deref(),
                case.content_type.as_deref(),
                case.content.as_deref().map(str::as_bytes),
            ),
            Predicate::File => {
                case.purpose.as_deref() == Some("batch")
                    && valid_content_type(case.content_type.as_deref())
                    && case.file.is_some()
            }
        })
        .collect();
    json!(results)
}

fn default_response(body: &str) -> FileResponse {
    FileResponse {
        status_code: 200,
        headers: Headers::new(),
        body: body.as_bytes().to_vec(),
        url: Some(String::from("https://mock.invalid/v1/files")),
    }
}

fn base_defaults(input: &BaseDefaultsInput) -> Value {
    let config = Config::default();
    let method = config.file_upload_http_method();
    let next = config.transform_list_files_next_request(
        &default_response(&input.response_body),
        &input.optional_params,
        &input.litellm_params,
    );
    json!({"http_method": method.as_str(), "next_request": next})
}

fn file_create(data: &FileData) -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: Some(data.filename.clone()),
            content_type: None,
            data: Vec::new(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn file_url_forwarding(input: &FileUrlInput) -> Result<Value, String> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let config = Config {
        complete_url: CompleteUrl::Recording(Arc::clone(&calls)),
    };
    let api_key = SecretString::from(input.api_key.clone());
    let data = file_create(&input.data);
    let url = config
        .get_complete_file_url(FileUrlRequest {
            api_base: Some(input.api_base.as_str()),
            api_key: Some(&api_key),
            model: input.model.as_str(),
            optional_params: &input.optional_params,
            litellm_params: &input.litellm_params,
            data: &data,
        })
        .map_err(|error| error.to_string())?;
    let recorded = calls.lock().map_err(|error| error.to_string())?.clone();
    let default_config = Config::default();
    let missing = match default_config.get_complete_file_url(FileUrlRequest {
        api_base: None,
        api_key: Some(&api_key),
        model: input.model.as_str(),
        optional_params: &input.optional_params,
        litellm_params: &input.litellm_params,
        data: &data,
    }) {
        Ok(built) => Value::String(built),
        Err(error) => Value::String(error.to_string()),
    };
    Ok(json!({"url": url, "calls": recorded, "missing_api_base": missing}))
}

fn decode_hex(chunks: &[String]) -> Result<Vec<Vec<u8>>, String> {
    let mut decoded = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        decoded.push(hex::decode(chunk).map_err(|error| error.to_string())?);
    }
    Ok(decoded)
}

fn collect_hex(stream: ByteStream) -> Vec<String> {
    let mut chunks = Vec::new();
    for chunk in stream {
        chunks.push(hex::encode(chunk));
    }
    chunks
}

fn stream_passthrough(input: &StreamPassthroughInput) -> Result<Value, String> {
    let config = Config::default();
    let mut logging = FileLogging::default();
    let result = config.transform_file_content_stream(
        Box::new(decode_hex(&input.chunks_hex)?.into_iter()),
        input.headers.clone(),
        StreamContext {
            request_url: input.request_url.as_str(),
            logging_obj: &mut logging,
            litellm_params: &input.litellm_params,
        },
    );
    let streamed = collect_hex(result.stream_iterator);
    let upload = Upload {
        chunks: decode_hex(&input.upload_chunks_hex)?,
    };
    let first = collect_hex(upload.iter_bytes());
    let second = collect_hex(upload.iter_bytes());
    Ok(json!({
        "stream": {"chunks": streamed, "headers": result.headers},
        "replay": {"first": first, "second": second},
    }))
}

fn run_functions(input: &Input) -> Result<Value, String> {
    match input {
        Input::BatchExtractedPredicate { cases } => {
            Ok(predicate_results(cases, Predicate::Content))
        }
        Input::BatchMetadataPredicate { cases } => Ok(predicate_results(cases, Predicate::File)),
        Input::BaseDefaults(fields) => Ok(base_defaults(fields)),
        Input::FileUrlForwarding(fields) => file_url_forwarding(fields),
        Input::StreamPassthroughAndReplay(fields) => stream_passthrough(fields),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        run_functions(input).map_err(|message| {
            Box::new(crate::helpers::HelperError {
                class: String::from("InvalidInput"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        })
    }
}
