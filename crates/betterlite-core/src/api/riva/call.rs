pub mod error;
pub mod logging;
mod options;
mod sdk;
mod stream;

mod tests;

use std::time::Duration;

use betterlite_context::non_default_params::Params;
use betterlite_models::call_helpers::headers::ProviderHeaderProjection;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_pyhost::blocking_work::WorkTracker;
use betterlite_pyhost::host::PythonHost;
use indexmap::IndexMap;
use pyo3::Python;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::SecretString;

use self::error::RivaCallError;
use self::logging::{RECOGNITION_CONFIG, RivaEvent, RivaLogger, RivaPostCall, RivaPreCall};
use self::options::{Resolved, endpoint, resolve};
use self::sdk::prepare_attached;
use self::stream::stream_attached;
use crate::api::riva::audio::{ResampledAudio, decode_riva_audio, decode_riva_audio_attached};
use crate::api::riva::records::response::{FinalResult, build_transcription_response};

// LiteLLM's custom_llm_provider for the NVIDIA Riva transcription handler
pub const PROVIDER: &str = "nvidia_riva";

// The handler's arguments after B047 resolved the endpoint and the key
pub struct RivaCall<'call> {
    pub model: &'call str,
    // get_audio_file_name of the original file, which post_call logs
    pub input: &'call str,
    pub file: &'call ProcessedAudioFile,
    pub optional_params: &'call Params,
    pub atranscription: bool,
    pub timeout: Duration,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
}

// _run_sync on a thread that already holds the interpreter
pub fn transcribe_attached(
    py: Python<'_>,
    call: &RivaCall<'_>,
    logger: &mut dyn RivaLogger,
) -> Result<TranscriptionReply, RivaCallError> {
    let uri = endpoint(call.api_base)?;
    let resampled = decode_riva_audio_attached(py, &call.file.file_content)?;
    let resolved = resolve(call.optional_params)?;
    let prepared = prepare_attached(py, &resolved, uri, call.api_key)?;
    let logged = json_records(&resolved.request.recognition_config)?;
    drop(logger.record(RivaEvent::PreCall(RivaPreCall {
        api_base: uri.to_owned(),
        atranscription: call.atranscription,
        recognition_config: logged,
        nvcf_function_id_set: resolved.function_id.is_some(),
        use_ssl: resolved.use_ssl_logged.clone(),
    })));
    let final_results = stream_attached(py, &prepared, &resampled.pcm_bytes, call.timeout)?;
    finish(call, &resolved, &resampled, &final_results, logger)
}

// async_audio_transcriptions: the decode and both SDK halves run through the tracked blocking pool, and atranscription=True reaches the log record from this entry alone
pub async fn transcribe(
    host: &PythonHost,
    tracker: &WorkTracker,
    deadline: Duration,
    call: &RivaCall<'_>,
    logger: &mut (dyn RivaLogger + Send),
) -> Result<TranscriptionReply, RivaCallError> {
    let uri = endpoint(call.api_base)?;
    let resampled =
        decode_riva_audio(host, tracker, deadline, call.file.file_content.clone()).await?;
    let resolved = resolve(call.optional_params)?;
    let logged = json_records(&resolved.request.recognition_config)?;
    drop(logger.record(RivaEvent::PreCall(RivaPreCall {
        api_base: uri.to_owned(),
        atranscription: true,
        recognition_config: logged,
        nvcf_function_id_set: resolved.function_id.is_some(),
        use_ssl: resolved.use_ssl_logged.clone(),
    })));
    let owned = resolved.clone();
    let owned_uri = uri.to_owned();
    let owned_key = call.api_key.cloned();
    let prepared = tracker
        .run(host, deadline, move |py| {
            prepare_attached(py, &owned, &owned_uri, owned_key.as_ref())
        })
        .await
        .map_err(RivaCallError::Work)??;
    let pcm = resampled.pcm_bytes.clone();
    let timeout = call.timeout;
    let final_results = tracker
        .run(host, deadline, move |py| {
            stream_attached(py, &prepared, &pcm, timeout)
        })
        .await
        .map_err(RivaCallError::Work)??;
    finish(call, &resolved, &resampled, &final_results, logger)
}

// The public response goes back, and B004's conversion reads the hidden model, provider and duration
fn finish(
    call: &RivaCall<'_>,
    resolved: &Resolved,
    resampled: &ResampledAudio,
    final_results: &[FinalResult],
    logger: &mut dyn RivaLogger,
) -> Result<TranscriptionReply, RivaCallError> {
    let duration = resampled
        .duration_seconds
        .to_f64()
        .ok_or(RivaCallError::Duration)?;
    let response = build_transcription_response(
        final_results,
        &resolved.request.response_format,
        Some(duration),
        resolved.request.timestamp_granularities.as_deref(),
    )?;
    let encoded =
        serde_json::to_value(&response).map_err(|source| RivaCallError::Json { source })?;
    let original_response =
        serde_json::from_value(encoded).map_err(|source| RivaCallError::Json { source })?;
    let recognition_config = json_records(&resolved.request.recognition_config)?;
    drop(logger.record(RivaEvent::PostCall(RivaPostCall {
        input: Json::Text(call.input.to_owned()),
        original_response: original_response.clone(),
        recognition_config,
    })));
    Ok(TranscriptionReply {
        response,
        original_response,
        provider_headers: ProviderHeaderProjection::default(),
        model: Some(call.model.to_owned()),
        custom_llm_provider: PROVIDER,
        duration: StoredDuration::Known(Some(resampled.duration_seconds)),
    })
}

fn json_records<T>(value: &T) -> Result<IndexMap<String, Json>, RivaCallError>
where
    T: serde::Serialize + ?Sized,
{
    let encoded = serde_json::to_value(value).map_err(|source| RivaCallError::Json { source })?;
    let Ok(fields) = serde_json::from_value::<IndexMap<String, Json>>(encoded) else {
        return Err(RivaCallError::Malformed {
            message: RECOGNITION_CONFIG.to_owned(),
        });
    };
    Ok(fields)
}
