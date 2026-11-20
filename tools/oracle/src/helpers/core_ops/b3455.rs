mod fakes;
mod inputs;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_core::rag::ingestion::store::{StoreRequest, StoreResult};
use betterlite_core::rag::ingestion::{IngestionPorts, IngestionState};
use betterlite_core::rag::vertex_ingestion::VertexIngestion;
use betterlite_core::rag::vertex_ingestion::error::VertexIngestionError;
use betterlite_core::rag::vertex_ingestion::ports::VertexIngestionPorts;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use fakes::{NoCredentials, NoFetch, NoOcr, ScriptedHttp, ScriptedToken};
use inputs::{FileInput, Input, ingestion_options, vector_store_options};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let options = ingestion_options(&input.options).map_err(invalid_input)?;
        let store_options = vector_store_options(&input.options).map_err(invalid_input)?;
        let content = match &input.file.content_base64 {
            None => None,
            Some(encoded) => Some(
                BASE64
                    .decode(encoded)
                    .map_err(|error| invalid_input(error.to_string()))?,
            ),
        };
        let token = Arc::new(ScriptedToken::default());
        let http = Arc::new(ScriptedHttp::new(
            input
                .responses
                .iter()
                .map(|reply| (reply.status, reply.body.clone()))
                .collect(),
        ));
        let state = IngestionState::new(
            options,
            None,
            IngestionPorts {
                ocr: Arc::new(NoOcr),
                uploader: Arc::new(NoFetch),
                credentials: Arc::new(NoCredentials),
            },
        )
        .map_err(|error| invalid_input(error.to_string()))?;
        let ingestion = VertexIngestion::new(
            state,
            store_options,
            &VertexSettings::default(),
            VertexIngestionPorts {
                token: Arc::clone(&token),
                http: Arc::clone(&http),
            },
        );
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
        Ok((corpus, file)) => (optional(corpus), optional(file), Value::Null),
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

fn optional(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::String)
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
