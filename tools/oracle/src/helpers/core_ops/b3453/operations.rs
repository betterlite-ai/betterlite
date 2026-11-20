use std::sync::{Arc, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_core::rag::ingestion::store::{RagStore as _, StoreRequest};
use betterlite_core::rag::openai_ingestion::OpenAiIngestion;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::fakes::{CallRecorder, ScriptedFile, ScriptedStore, ScriptedUpload, call_json};
use super::inputs::{StoreInput, build_state};

pub(super) fn store(input: &StoreInput) -> HelperResult {
    let recorder = Arc::new(CallRecorder::default());
    let ingestion = scripted_ingestion(input, &recorder)?;
    let content = decoded_content(input)?;
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let stored = runtime
        .block_on(ingestion.store(StoreRequest {
            file_content: content.as_deref(),
            filename: input.filename.as_deref(),
            content_type: input.content_type.as_deref(),
            chunks: &[],
            embeddings: None,
            existing_file_id: input.existing_file_id.as_deref(),
        }))
        .or_value_error()?;
    let calls = recorder
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(call_json)
        .collect::<Result<Vec<Value>, _>>()?;
    Ok(json!({
        "calls": calls,
        "result": [stored.0, stored.1],
    }))
}

fn scripted_ingestion(
    input: &StoreInput,
    recorder: &Arc<CallRecorder>,
) -> Result<OpenAiIngestion, Box<HelperError>> {
    let ingestion = OpenAiIngestion {
        state: build_state(input)?,
        vector_store: Arc::new(ScriptedStore {
            recorder: Arc::clone(recorder),
            response: input
                .create_response
                .as_ref()
                .and_then(|response| response.id.clone()),
            failure: input.create_error.clone(),
        }),
        vector_file: Arc::new(ScriptedFile {
            recorder: Arc::clone(recorder),
            failure: input.attach_error.clone(),
        }),
        file_upload: Arc::new(ScriptedUpload {
            recorder: Arc::clone(recorder),
            id: input
                .upload_id
                .clone()
                .unwrap_or_else(|| "file-uploaded".to_owned()),
            failure: input.upload_error.clone(),
        }),
    };
    Ok(ingestion)
}

fn decoded_content(input: &StoreInput) -> Result<Option<Vec<u8>>, Box<HelperError>> {
    input
        .file_content_base64
        .as_deref()
        .map(|encoded| BASE64.decode(encoded).or_value_error())
        .transpose()
}
