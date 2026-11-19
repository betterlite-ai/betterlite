use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::credentials::CredentialValues;
use betterlite_config::rag::document::ingest::{RAGIngestEmbeddingOptions, RAGIngestOCROptions};
use betterlite_core::rag::ingestion::config::{IngestionOptions, VectorStoreConfig};
use betterlite_core::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use betterlite_core::rag::ingestion::ocr::{
    OcrDocument, OcrError, OcrOperation, OcrRequest, OcrResponse,
};
use betterlite_core::rag::ingestion::upload::{FetchedResponse, FileData, UrlFetch, UrlFetchError};
use betterlite_core::rag::ingestion::{IngestionPorts, IngestionState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use http::HeaderMap;
use http::HeaderValue;
use http::header::CONTENT_TYPE;
use serde::Deserialize;
use serde_json::value::{RawValue, to_raw_value};
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;

#[derive(Debug, Clone, Default, Deserialize)]
struct OptionsInput {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    ocr: Option<RAGIngestOCROptions>,
    #[serde(default)]
    chunking_strategy: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    embedding: Option<RAGIngestEmbeddingOptions>,
    #[serde(default)]
    vector_store: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Deserialize)]
struct CredentialInput {
    name: String,
    values: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct FileDataInput {
    filename: String,
    content_base64: String,
    content_type: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FetchInput {
    status: u16,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    body_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    Init {
        options: OptionsInput,
        #[serde(default)]
        credentials: Vec<CredentialInput>,
    },
    Upload {
        options: OptionsInput,
        #[serde(default)]
        file_data: Option<FileDataInput>,
        #[serde(default)]
        file_url: Option<String>,
        #[serde(default)]
        file_id: Option<String>,
        #[serde(default)]
        fetch: Option<FetchInput>,
    },
    Ocr {
        options: OptionsInput,
        #[serde(default)]
        file_content_base64: Option<String>,
        #[serde(default)]
        content_type: Option<String>,
        #[serde(default)]
        pages: Vec<String>,
    },
    Chunk {
        options: OptionsInput,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        file_content_base64: Option<String>,
        #[serde(default)]
        ocr_was_used: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Init {
                options,
                credentials,
            } => init(options, credentials),
            Input::Upload {
                options,
                file_data,
                file_url,
                file_id,
                fetch,
            } => upload(options, file_data, file_url, file_id, fetch),
            Input::Ocr {
                options,
                file_content_base64,
                content_type,
                pages,
            } => ocr(options, file_content_base64, content_type, pages),
            Input::Chunk {
                options,
                text,
                file_content_base64,
                ocr_was_used,
            } => chunk(options, text, file_content_base64, *ocr_was_used),
        }
    }
}

#[derive(Default)]
struct RecordingOcr {
    calls: Mutex<Vec<OcrRequest>>,
    pages: Vec<String>,
}

impl RecordingOcr {
    #[must_use]
    fn with_pages(pages: &[String]) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            pages: pages.to_vec(),
        }
    }
}

#[async_trait]
impl OcrOperation for RecordingOcr {
    async fn aocr(&self, request: OcrRequest) -> Result<OcrResponse, OcrError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(OcrResponse::from_pages(self.pages.clone()))
    }
}

struct RecordingFetch {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl RecordingFetch {
    fn answer(input: Option<&FetchInput>) -> Result<Self, Box<HelperError>> {
        let Some(fetch) = input else {
            return Ok(Self {
                status: 200,
                content_type: None,
                body: Vec::new(),
            });
        };
        Ok(Self {
            status: fetch.status,
            content_type: fetch.content_type.clone(),
            body: decode_base64(&fetch.body_base64)?,
        })
    }

    fn headers(&self) -> Result<HeaderMap, UrlFetchError> {
        let Some(content_type) = &self.content_type else {
            return Ok(HeaderMap::new());
        };
        let value =
            HeaderValue::from_str(content_type).map_err(|_error| UrlFetchError::Failed {
                message: "the fake content type is not a header value".to_owned(),
            })?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, value);
        Ok(headers)
    }
}

#[async_trait]
impl UrlFetch for RecordingFetch {
    async fn fetch(&self, _url: &str) -> Result<FetchedResponse, UrlFetchError> {
        Ok(FetchedResponse {
            status: self.status,
            headers: self.headers()?,
            body: self.body.clone(),
        })
    }
}

struct FixedCredentials {
    values: BTreeMap<String, CredentialValues>,
}

impl CredentialLookup for FixedCredentials {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        self.values
            .get(credential_name)
            .cloned()
            .ok_or_else(|| CredentialLookupError::NotFound {
                credential_name: credential_name.to_owned(),
            })
    }
}

fn raw_map(
    entries: Option<BTreeMap<String, Value>>,
) -> Result<Option<BTreeMap<String, Box<RawValue>>>, Box<HelperError>> {
    let Some(entries) = entries else {
        return Ok(None);
    };
    let mut raws: BTreeMap<String, Box<RawValue>> = BTreeMap::new();
    for (key, value) in entries {
        raws.insert(key, to_raw_value(&value)?);
    }
    Ok(Some(raws))
}

fn options_value(input: &OptionsInput) -> Result<IngestionOptions, Box<HelperError>> {
    Ok(IngestionOptions {
        name: input.name.clone(),
        ocr: input.ocr.clone(),
        chunking_strategy: raw_map(input.chunking_strategy.clone())?,
        embedding: input.embedding.clone(),
        vector_store: raw_map(input.vector_store.clone())?,
    })
}

fn credential_values(
    inputs: &[CredentialInput],
) -> Result<BTreeMap<String, CredentialValues>, Box<HelperError>> {
    let mut values = BTreeMap::new();
    for input in inputs {
        let parsed: CredentialValues = serde_json::from_value(input.values.clone())?;
        values.insert(input.name.clone(), parsed);
    }
    Ok(values)
}

fn build(
    options_input: &OptionsInput,
    credential_inputs: &[CredentialInput],
    ocr: Arc<dyn OcrOperation>,
    fetch: Arc<dyn UrlFetch>,
) -> Result<IngestionState, Box<HelperError>> {
    IngestionState::new(
        options_value(options_input)?,
        None,
        IngestionPorts {
            ocr,
            uploader: fetch,
            credentials: Arc::new(FixedCredentials {
                values: credential_values(credential_inputs)?,
            }),
        },
    )
    .or_value_error()
}

fn init(options_input: &OptionsInput, credential_inputs: &[CredentialInput]) -> HelperResult {
    let built = build(
        options_input,
        credential_inputs,
        Arc::new(RecordingOcr::default()),
        Arc::new(RecordingFetch::answer(None)?),
    )?;
    let suffix = built.ingest_id.strip_prefix("ingest_").unwrap_or_default();
    Ok(json!({
        "ingest_id": {
            "prefixed": built.ingest_id.starts_with("ingest_"),
            "suffix_len": suffix.len(),
        },
        "ingest_name": built.ingest_name,
        "ocr_config": built.ocr_config,
        "chunking_strategy": raw_value(&built.chunking_strategy)?,
        "embedding_config": built.embedding_config,
        "vector_store_config": raw_values(&built.vector_store_config)?,
        "supports_existing_file_id": IngestionState::SUPPORTS_EXISTING_FILE_ID,
    }))
}

fn upload(
    options_input: &OptionsInput,
    file_data: &Option<FileDataInput>,
    file_url: &Option<String>,
    file_id: &Option<String>,
    fetch: &Option<FetchInput>,
) -> HelperResult {
    let recorder = Arc::new(RecordingFetch::answer(fetch.as_ref())?);
    let built = build(
        options_input,
        &[],
        Arc::new(RecordingOcr::default()),
        recorder,
    )?;
    let data = match file_data {
        Some(data) => Some(FileData {
            filename: data.filename.clone(),
            content: decode_base64(&data.content_base64)?,
            content_type: data.content_type.clone(),
        }),
        None => None,
    };
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let uploaded = runtime
        .block_on(built.upload(data, file_url.as_deref(), file_id.as_deref()))
        .or_value_error()?;
    Ok(json!({
        "filename": uploaded.filename,
        "file_content_base64": uploaded
            .file_content
            .map(|content| BASE64.encode(content)),
        "content_type": uploaded.content_type,
        "existing_file_id": uploaded.existing_file_id,
    }))
}

fn ocr(
    options_input: &OptionsInput,
    file_content_base64: &Option<String>,
    content_type: &Option<String>,
    pages: &[String],
) -> HelperResult {
    let recorder = Arc::new(RecordingOcr::with_pages(pages));
    let built = build(
        options_input,
        &[],
        recorder.clone(),
        Arc::new(RecordingFetch::answer(None)?),
    )?;
    let content = match file_content_base64 {
        Some(encoded) => Some(decode_base64(encoded)?),
        None => None,
    };
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let text = runtime
        .block_on(built.ocr(content.as_deref(), content_type.as_deref()))
        .or_value_error()?;
    let calls: Vec<Value> = recorder
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|call| {
            json!({
                "model": call.model,
                "document": document_value(&call.document),
            })
        })
        .collect();
    Ok(json!({ "text": text, "calls": calls }))
}

fn chunk(
    options_input: &OptionsInput,
    text: &Option<String>,
    file_content_base64: &Option<String>,
    ocr_was_used: bool,
) -> HelperResult {
    let built = build(
        options_input,
        &[],
        Arc::new(RecordingOcr::default()),
        Arc::new(RecordingFetch::answer(None)?),
    )?;
    let content = match file_content_base64 {
        Some(encoded) => Some(decode_base64(encoded)?),
        None => None,
    };
    let chunks = built
        .chunk(text.as_deref(), content.as_deref(), ocr_was_used)
        .or_value_error()?;
    Ok(json!({ "chunks": chunks }))
}

fn document_value(document: &OcrDocument) -> Value {
    let mut fields = Map::new();
    fields.insert(
        "type".to_owned(),
        Value::String(document.kind.wire_name().to_owned()),
    );
    fields.insert(
        document.kind.wire_name().to_owned(),
        Value::String(document.data_url.clone()),
    );
    Value::Object(fields)
}

fn raw_value(raw: &RawValue) -> Result<Value, Box<HelperError>> {
    Ok(serde_json::from_str(raw.get())?)
}

fn raw_values(config: &VectorStoreConfig) -> Result<Value, Box<HelperError>> {
    let mut fields = Map::new();
    for (key, value) in config {
        fields.insert(key.clone(), raw_value(value)?);
    }
    Ok(Value::Object(fields))
}

fn decode_base64(text: &str) -> Result<Vec<u8>, Box<HelperError>> {
    BASE64.decode(text).or_value_error()
}
