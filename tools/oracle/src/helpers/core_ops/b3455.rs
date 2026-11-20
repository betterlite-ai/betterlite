mod fakes;
mod inputs;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::rag::document::ingest::VertexAIVectorStoreOptions;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_core::rag::ingestion::IngestionPorts;
use betterlite_core::rag::ingestion::IngestionState;
use betterlite_core::rag::ingestion::config::IngestionOptions;
use betterlite_core::rag::ingestion::store::{StoreRequest, StoreResult};
use betterlite_core::rag::vertex_ingestion::VertexIngestion;
use betterlite_core::rag::vertex_ingestion::error::VertexIngestionError;
use betterlite_core::rag::vertex_ingestion::ports::VertexIngestionPorts;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use fakes::{NoCredentials, NoFetch, NoOcr, ScriptedHttp, ScriptedToken};
use inputs::{FileInput, Input};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let content = file_content(&input.file).map_err(invalid_input)?;
        let token = Arc::new(ScriptedToken::default());
        let http = Arc::new(ScriptedHttp::new(replies(input)));
        let ingestion = vertex_ingestion(input, &token, &http).map_err(invalid_input)?;
        let runtime = Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .or_value_error()?;
        let (outcome, slept) =
            runtime.block_on(store_and_time(&ingestion, &input.file, content.as_deref()));
        Ok(projection(http.requests(), slept, token.calls(), outcome))
    }
}

fn replies(input: &Input) -> Vec<(u16, String)> {
    input
        .responses
        .iter()
        .map(|reply| (reply.status, reply.body.clone()))
        .collect()
}

fn file_content(file: &FileInput) -> Result<Option<Vec<u8>>, String> {
    let Some(encoded) = &file.content_base64 else {
        return Ok(None);
    };
    BASE64
        .decode(encoded)
        .map(Some)
        .map_err(|error| error.to_string())
}

// LiteLLM reads the same object twice: once as ingest options and once as the vector store config
fn vertex_ingestion(
    input: &Input,
    token: &Arc<ScriptedToken>,
    http: &Arc<ScriptedHttp>,
) -> Result<VertexIngestion, String> {
    let options: IngestionOptions =
        serde_json::from_value(input.options.clone()).map_err(|error| error.to_string())?;
    let empty = Value::Object(serde_json::Map::default());
    let store = input.options.get("vector_store").unwrap_or(&empty);
    let store_options: VertexAIVectorStoreOptions =
        serde_json::from_value(store.clone()).map_err(|error| error.to_string())?;
    let state = IngestionState::new(
        options,
        None,
        IngestionPorts {
            ocr: Arc::new(NoOcr),
            uploader: Arc::new(NoFetch),
            credentials: Arc::new(NoCredentials),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(VertexIngestion::new(
        state,
        store_options,
        &VertexSettings::default(),
        VertexIngestionPorts {
            token: Arc::clone(token),
            http: Arc::clone(http),
        },
    ))
}

// the paused clock jumps to the next timer while the runtime has nothing else to do
async fn store_and_time(
    ingestion: &VertexIngestion,
    file: &FileInput,
    content: Option<&[u8]>,
) -> (Result<StoreResult, VertexIngestionError>, Duration) {
    let chunks: [String; 0] = [];
    let request = StoreRequest {
        file_content: content,
        filename: file.filename.as_deref(),
        content_type: file.content_type.as_deref(),
        chunks: &chunks,
        embeddings: None,
        existing_file_id: None,
    };
    let started = tokio::time::Instant::now();
    let outcome = ingestion.store(request).await;
    (outcome, started.elapsed())
}

fn projection(
    requests: Vec<Value>,
    slept: Duration,
    token_calls: Vec<Value>,
    outcome: Result<StoreResult, VertexIngestionError>,
) -> Value {
    let (corpus_id, file_id, failure) = match outcome {
        Ok((corpus, file)) => (
            corpus.map_or(Value::Null, Value::String),
            file.map_or(Value::Null, Value::String),
            Value::Null,
        ),
        Err(error) => (Value::Null, Value::Null, Value::String(error.to_string())),
    };
    json!({
        "requests": requests,
        "sleep_seconds": slept.as_secs_f64(),
        "token_calls": token_calls,
        "corpus_id": corpus_id,
        "file_id": file_id,
        "failure": failure,
    })
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
