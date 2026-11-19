use std::collections::BTreeMap;
use std::sync::Arc;

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

use super::fakes::FixedCredentials;

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
