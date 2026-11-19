use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::credentials::CredentialValues;
use betterlite_config::rag::document::ingest::{RAGIngestEmbeddingOptions, RAGIngestOCROptions};
use betterlite_core::rag::ingestion::config::IngestionOptions;
use betterlite_core::rag::ingestion::ocr::OcrOperation;
use betterlite_core::rag::ingestion::upload::UrlFetch;
use betterlite_core::rag::ingestion::{IngestionPorts, IngestionState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::{RawValue, to_raw_value};

use super::fakes::{FixedCredentials, RecordingFetch, RecordingOcr};

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct OptionsInput {
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
pub(crate) struct CredentialInput {
    name: String,
    values: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileDataInput {
    pub(super) filename: String,
    pub(super) content_base64: String,
    pub(super) content_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FetchInput {
    pub(super) status: u16,
    pub(super) content_type: Option<String>,
    pub(super) body_base64: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InitInput {
    pub(super) options: OptionsInput,
    #[serde(default)]
    pub(super) credentials: Vec<CredentialInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UploadInput {
    pub(super) options: OptionsInput,
    #[serde(default)]
    pub(super) file_data: Option<FileDataInput>,
    #[serde(default)]
    pub(super) file_url: Option<String>,
    #[serde(default)]
    pub(super) file_id: Option<String>,
    #[serde(default)]
    pub(super) fetch: Option<FetchInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OcrInput {
    pub(super) options: OptionsInput,
    #[serde(default)]
    pub(super) file_content_base64: Option<String>,
    #[serde(default)]
    pub(super) content_type: Option<String>,
    #[serde(default)]
    pub(super) pages: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkInput {
    pub(super) options: OptionsInput,
    #[serde(default)]
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) file_content_base64: Option<String>,
    #[serde(default)]
    pub(super) ocr_was_used: bool,
}

impl ChunkInput {
    pub(super) fn chunks(&self) -> Result<Vec<String>, Box<HelperError>> {
        let built = state(&self.options, &[])?;
        let content = decode_content(self.file_content_base64.as_deref())?;
        built
            .chunk(self.text.as_deref(), content.as_deref(), self.ocr_was_used)
            .or_value_error()
    }
}

fn raw_map(
    entries: Option<BTreeMap<String, Value>>,
) -> Result<Option<BTreeMap<String, Box<RawValue>>>, Box<HelperError>> {
    let Some(raw_entries) = entries else {
        return Ok(None);
    };
    let mut raws: BTreeMap<String, Box<RawValue>> = BTreeMap::new();
    for (key, value) in raw_entries {
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

pub(super) fn build(
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

pub(super) fn state(
    options: &OptionsInput,
    credentials: &[CredentialInput],
) -> Result<IngestionState, Box<HelperError>> {
    build(
        options,
        credentials,
        Arc::new(RecordingOcr::default()),
        Arc::new(RecordingFetch::answer(None)?),
    )
}

pub(super) fn decode_content(encoded: Option<&str>) -> Result<Option<Vec<u8>>, Box<HelperError>> {
    let Some(encoded) = encoded else {
        return Ok(None);
    };
    Ok(Some(BASE64.decode(encoded).or_value_error()?))
}
