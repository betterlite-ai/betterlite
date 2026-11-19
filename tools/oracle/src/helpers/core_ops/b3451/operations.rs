use std::sync::{Arc, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_core::rag::ingestion::IngestionState;
use betterlite_core::rag::ingestion::config::VectorStoreConfig;
use betterlite_core::rag::ingestion::ocr::{OcrDocument, OcrOperation};
use betterlite_core::rag::ingestion::upload::FileData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;

use super::fakes::{RecordingFetch, RecordingOcr};
use super::inputs::{ChunkInput, InitInput, OcrInput, UploadInput, build, decode_content, state};

pub(super) fn init(input: &InitInput) -> HelperResult {
    let built = state(&input.options, &input.credentials)?;
    let suffix = built.ingest_id.strip_prefix("ingest_").unwrap_or_default();
    let mut report = Map::new();
    report.insert(
        "ingest_id".to_owned(),
        json!({
            "prefixed": built.ingest_id.starts_with("ingest_"),
            "suffix_len": suffix.len(),
        }),
    );
    report.insert("ingest_name".to_owned(), json!(built.ingest_name));
    report.insert("ocr_config".to_owned(), json!(built.ocr_config));
    report.insert(
        "chunking_strategy".to_owned(),
        serde_json::from_str::<Value>(built.chunking_strategy.get())?,
    );
    report.insert("embedding_config".to_owned(), json!(built.embedding_config));
    report.insert(
        "vector_store_config".to_owned(),
        raw_values(&built.vector_store_config)?,
    );
    report.insert(
        "supports_existing_file_id".to_owned(),
        json!(IngestionState::SUPPORTS_EXISTING_FILE_ID),
    );
    Ok(Value::Object(report))
}

pub(super) fn upload(input: &UploadInput) -> HelperResult {
    let recorder = Arc::new(RecordingFetch::answer(input.fetch.as_ref())?);
    let built = build(
        &input.options,
        &[],
        Arc::new(RecordingOcr::default()),
        recorder,
    )?;
    let data = match &input.file_data {
        Some(data) => Some(FileData {
            filename: data.filename.clone(),
            content: BASE64.decode(&data.content_base64).or_value_error()?,
            content_type: data.content_type.clone(),
        }),
        None => None,
    };
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let uploaded = runtime
        .block_on(built.upload(data, input.file_url.as_deref(), input.file_id.as_deref()))
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

pub(super) fn ocr(input: &OcrInput) -> HelperResult {
    let recorder = Arc::new(RecordingOcr::with_pages(&input.pages));
    let ocr_port: Arc<dyn OcrOperation> = Arc::<RecordingOcr>::clone(&recorder);
    let built = build(
        &input.options,
        &[],
        ocr_port,
        Arc::new(RecordingFetch::answer(None)?),
    )?;
    let content = decode_content(input.file_content_base64.as_deref())?;
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let text = runtime
        .block_on(built.ocr(content.as_deref(), input.content_type.as_deref()))
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

pub(super) fn chunk(input: &ChunkInput) -> HelperResult {
    let built = state(&input.options, &[])?;
    let content = decode_content(input.file_content_base64.as_deref())?;
    let chunks = built
        .chunk(
            input.text.as_deref(),
            content.as_deref(),
            input.ocr_was_used,
        )
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
