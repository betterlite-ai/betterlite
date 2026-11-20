mod transport;

use std::collections::BTreeMap;
use std::mem::take;
use std::sync::{Arc, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::credentials::CredentialValues;
use betterlite_config::sections::studio_identity::DEFAULT_API_BASE;
use betterlite_core::rag::gemini_ingestion::port::GeminiTransport;
use betterlite_core::rag::gemini_ingestion::{
    GeminiDefaults, GeminiIngestion, GeminiIngestionError,
};
use betterlite_core::rag::ingestion::IngestionPorts;
use betterlite_core::rag::ingestion::IngestionState;
use betterlite_core::rag::ingestion::config::IngestionOptions;
use betterlite_core::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use betterlite_core::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use betterlite_core::rag::ingestion::store::{RagStore as _, StoreRequest};
use betterlite_core::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::{RawValue, to_raw_value};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use transport::{Scripted, Step};

const KEY_NAMES: [&str; 2] = ["GOOGLE_API_KEY", "GEMINI_API_KEY"];

#[derive(Debug, Default, Deserialize)]
struct OptionsInput {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    chunking_strategy: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    vector_store: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    options: OptionsInput,
    #[serde(default)]
    file_content_base64: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    script: Vec<Step>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        store(input)
    }
}

struct NoOcr;

#[async_trait]
impl OcrOperation for NoOcr {
    async fn aocr(&self, _request: OcrRequest) -> Result<OcrResponse, OcrError> {
        Err(OcrError::Failed {
            message: "the Gemini store never runs OCR".to_owned(),
        })
    }
}

struct NoFetch;

#[async_trait]
impl UrlFetch for NoFetch {
    async fn fetch(&self, _url: &str) -> Result<FetchedResponse, UrlFetchError> {
        Err(UrlFetchError::Failed {
            message: "the Gemini store never fetches a URL".to_owned(),
        })
    }
}

struct NoCredentials;

impl CredentialLookup for NoCredentials {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        Err(CredentialLookupError::NotFound {
            credential_name: credential_name.to_owned(),
        })
    }
}

fn store(input: &Input) -> HelperResult {
    let content = decode_content(input.file_content_base64.as_deref())?;
    let transport = Arc::new(Scripted::new(input.script.clone()));
    let port: Arc<dyn GeminiTransport> = Arc::<Scripted>::clone(&transport);
    let ingestion = GeminiIngestion {
        state: state(&input.options)?,
        defaults: defaults(&input.environment),
        transport: port,
    };
    let request = StoreRequest {
        file_content: content.as_deref(),
        filename: input.filename.as_deref(),
        content_type: input.content_type.as_deref(),
        chunks: &[],
        embeddings: None,
        existing_file_id: None,
    };
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let (vector_store_id, file_id) = runtime
        .block_on(ingestion.store(request))
        .map_err(|error| gemini_error(&error))?;
    let sent = take(
        &mut *transport
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    Ok(json!({
        "requests": sent,
        "vector_store_id": vector_store_id,
        "file_id": file_id,
    }))
}

fn state(options: &OptionsInput) -> Result<IngestionState, Box<HelperError>> {
    IngestionState::new(
        IngestionOptions {
            name: options.name.clone(),
            ocr: None,
            chunking_strategy: raw_map(options.chunking_strategy.as_ref())?,
            embedding: None,
            vector_store: raw_map(options.vector_store.as_ref())?,
        },
        None,
        IngestionPorts {
            ocr: Arc::new(NoOcr),
            uploader: Arc::new(NoFetch),
            credentials: Arc::new(NoCredentials),
        },
    )
    .or_value_error()
}

fn decode_content(encoded: Option<&str>) -> Result<Option<Vec<u8>>, Box<HelperError>> {
    let Some(value) = encoded else {
        return Ok(None);
    };
    Ok(Some(BASE64.decode(value).or_value_error()?))
}

fn raw_map(
    entries: Option<&BTreeMap<String, Value>>,
) -> Result<Option<BTreeMap<String, Box<RawValue>>>, Box<HelperError>> {
    let Some(raw_entries) = entries else {
        return Ok(None);
    };
    let mut raws: BTreeMap<String, Box<RawValue>> = BTreeMap::new();
    for (key, value) in raw_entries {
        raws.insert(key.clone(), to_raw_value(value)?);
    }
    Ok(Some(raws))
}

fn defaults(environment: &BTreeMap<String, String>) -> GeminiDefaults {
    let api_key = KEY_NAMES.iter().find_map(|name| {
        environment
            .get(*name)
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.clone()))
    });
    let api_base = environment
        .get("GEMINI_API_BASE")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    GeminiDefaults { api_key, api_base }
}

fn gemini_error(error: &GeminiIngestionError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "Exception".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
