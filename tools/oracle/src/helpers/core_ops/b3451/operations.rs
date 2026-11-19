use std::sync::{Arc, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_core::rag::ingestion::IngestionState;
use betterlite_core::rag::ingestion::config::VectorStoreConfig;
use betterlite_core::rag::ingestion::ocr::{OcrDocument, OcrOperation};
use betterlite_core::rag::ingestion::upload::FileData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;

use super::fakes::{RecordingFetch, RecordingOcr};
use super::inputs::{CredentialInput, FetchInput, FileDataInput, OptionsInput, build};

pub(super) fn init(
    options_input: &OptionsInput,
    credential_inputs: &[CredentialInput],
) -> HelperResult {
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
        "chunking_strategy": serde_json::from_str::<Value>(built.chunking_strategy.get())?,
        "embedding_config": built.embedding_config,
        "vector_store_config": raw_values(&built.vector_store_config)?,
        "supports_existing_file_id": IngestionState::SUPPORTS_EXISTING_FILE_ID,
    }))
}

pub(super) fn upload(
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
            content: BASE64.decode(&data.content_base64).or_value_error()?,
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

pub(super) fn ocr(
    options_input: &OptionsInput,
    file_content_base64: &Option<String>,
    content_type: &Option<String>,
    pages: &[String],
) -> HelperResult {
    let recorder = Arc::new(RecordingOcr::with_pages(pages));
    let ocr_port: Arc<dyn OcrOperation> = Arc::<RecordingOcr>::clone(&recorder);
    let built = build(
        options_input,
        &[],
        ocr_port,
        Arc::new(RecordingFetch::answer(None)?),
    )?;
    let content = match file_content_base64 {
        Some(encoded) => Some(BASE64.decode(encoded).or_value_error()?),
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

pub(super) fn chunk(
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
        Some(encoded) => Some(BASE64.decode(encoded).or_value_error()?),
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

fn raw_values(config: &VectorStoreConfig) -> Result<Value, Box<HelperError>> {
    let mut fields = Map::new();
    for (key, value) in config {
        fields.insert(key.clone(), serde_json::from_str::<Value>(value.get())?);
    }
    Ok(Value::Object(fields))
}
