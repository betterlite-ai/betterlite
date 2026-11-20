use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::credentials::CredentialValues;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::value::{RawValue, to_raw_value};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::transport::{Scripted, ScriptedReply, Sent, reply};
use crate::rag::gemini_ingestion::port::GeminiTransport;
use crate::rag::gemini_ingestion::{GeminiDefaults, GeminiIngestion, GeminiIngestionError};
use crate::rag::ingestion::config::IngestionOptions;
use crate::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use crate::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use crate::rag::ingestion::store::{RagStore as _, StoreRequest, StoreResult};
use crate::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};
use crate::rag::ingestion::{IngestionPorts, IngestionState};

pub(super) const KEY: &str = "secret-key";
pub(super) const BASE: &str = "https://gemini.test";
pub(super) const STORE: &str = "fileSearchStores/s";
pub(super) const NO_CHUNKS: &[String] = &[];

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

pub(super) struct Fixture {
    pub(super) ingestion: GeminiIngestion,
    pub(super) transport: Arc<Scripted>,
}

pub(super) fn fixtures(
    options: IngestionOptions,
    defaults: GeminiDefaults,
    replies: Vec<ScriptedReply>,
) -> Fixture {
    let transport = Arc::new(Scripted::new(replies));
    let state = IngestionState::new(
        options,
        None,
        IngestionPorts {
            ocr: Arc::new(NoOcr),
            uploader: Arc::new(NoFetch),
            credentials: Arc::new(NoCredentials),
        },
    )
    .unwrap();
    let port: Arc<dyn GeminiTransport> = Arc::<Scripted>::clone(&transport);
    Fixture {
        ingestion: GeminiIngestion {
            state,
            defaults,
            transport: port,
        },
        transport,
    }
}

pub(super) fn defaults() -> GeminiDefaults {
    GeminiDefaults {
        api_key: Some(SecretString::from(KEY.to_owned())),
        api_base: BASE.to_owned(),
    }
}

pub(super) fn keyless() -> GeminiDefaults {
    GeminiDefaults {
        api_key: None,
        api_base: BASE.to_owned(),
    }
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the case JSON parses")
}

fn object(text: &str) -> BTreeMap<String, Box<RawValue>> {
    let Json::Object(fields) = json(text) else {
        panic!("the case builds an object");
    };
    fields
        .into_iter()
        .map(|(key, value)| (key, to_raw_value(&value).unwrap()))
        .collect()
}

pub(super) fn options(name: Option<&str>, vector_store: &str) -> IngestionOptions {
    IngestionOptions {
        name: name.map(str::to_owned),
        ocr: None,
        chunking_strategy: None,
        embedding: None,
        vector_store: Some(object(vector_store)),
    }
}

pub(super) fn with_chunking(mut options: IngestionOptions, strategy: &str) -> IngestionOptions {
    options.chunking_strategy = Some(object(strategy));
    options
}

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

pub(super) async fn store(
    fixture: &Fixture,
    content: Option<&[u8]>,
    filename: Option<&str>,
    content_type: Option<&str>,
) -> Result<StoreResult, GeminiIngestionError> {
    let request = StoreRequest {
        file_content: content,
        filename,
        content_type,
        chunks: NO_CHUNKS,
        embeddings: None,
        existing_file_id: None,
    };
    fixture.ingestion.store(request).await
}

pub(super) fn header<'headers>(headers: &'headers HeaderMap, name: &str) -> &'headers str {
    headers
        .get(name)
        .unwrap_or_else(|| panic!("the {name} header"))
        .to_str()
        .unwrap()
}

pub(super) fn body_json(request: &Sent) -> Json {
    serde_json::from_slice(&request.body).unwrap()
}

pub(super) const fn gemini_only() -> &'static str {
    r#"{"custom_llm_provider":"gemini"}"#
}

pub(super) const fn with_store_id() -> &'static str {
    r#"{"custom_llm_provider":"gemini","vector_store_id":"fileSearchStores/s"}"#
}

pub(super) fn created_store() -> ScriptedReply {
    reply(200, &[], br#"{"name":"fileSearchStores/s"}"#)
}

pub(super) fn upload_url() -> ScriptedReply {
    reply(
        200,
        &[("x-goog-upload-url", "https://upload.test/session")],
        b"{}",
    )
}

pub(super) fn finalized(name: &str) -> ScriptedReply {
    reply(200, &[], format!(r#"{{"name":"{name}"}}"#).as_bytes())
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<(Level, String)>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), message.0));
    }
}

pub(super) fn capture<T, F>(call: F) -> (T, Vec<(Level, String)>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = registry().with(logs.clone());
    let value = with_default(subscriber, call);
    let captured = std::mem::take(&mut *logs.0.lock().unwrap_or_else(PoisonError::into_inner));
    (value, captured)
}
