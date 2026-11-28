use betterlite_core::api::riva::call::error::RivaCallError;
use betterlite_core::api::riva::call::logging::{RivaEvent, RivaLogger};
use betterlite_core::api::transcription_call::error::TranscriptionDispatchError as _;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_pyhost::host::PythonHost;
use indexmap::IndexMap;
use pyo3::PyResult;
use pyo3::types::PyAnyMethods as _;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::helper_error;

#[derive(Default)]
pub(super) struct RecordingLogger {
    pre: Option<LoggedPre>,
    post: Option<LoggedPost>,
}

struct LoggedPre {
    api_base: String,
    atranscription: bool,
    nvcf_function_id_set: bool,
    use_ssl: Json,
    recognition_config: IndexMap<String, Json>,
}

struct LoggedPost {
    input: Json,
    original_response: Json,
    recognition_config: IndexMap<String, Json>,
}

impl RivaLogger for RecordingLogger {
    fn record(&mut self, event: RivaEvent) -> Result<(), PrePostError> {
        match event {
            RivaEvent::PreCall(record) => {
                self.pre = Some(LoggedPre {
                    api_base: record.api_base,
                    atranscription: record.atranscription,
                    nvcf_function_id_set: record.nvcf_function_id_set,
                    use_ssl: record.use_ssl,
                    recognition_config: record.recognition_config,
                });
            }
            RivaEvent::PostCall(record) => {
                self.post = Some(LoggedPost {
                    input: record.input,
                    original_response: record.original_response,
                    recognition_config: record.recognition_config,
                });
            }
        }
        Ok(())
    }
}

// the fake SDK's snapshot of what the handler did, as its Python objects saw it
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct Probe {
    events: Vec<String>,
    auth_kwargs: Option<Json>,
    auth_args: Option<Json>,
    recognition: Option<Json>,
    endpointing_kwargs: Option<Json>,
    endpointing_copied: bool,
    interim_results: Option<Json>,
    config_is_recognition: bool,
    chunk_sizes: Vec<Vec<usize>>,
    timeout_seconds: Vec<Option<f64>>,
}

pub(super) fn read_probe(host: &PythonHost) -> Result<Probe, Box<HelperError>> {
    let text: String = host
        .attach(|py| -> PyResult<String> {
            let module = py.import("riva.client")?;
            module.call_method0("snapshot")?.extract()
        })
        .map_err(|error| helper_error("PythonError", error.to_string()))?;
    serde_json::from_str(&text).map_err(|error| {
        helper_error(
            "PythonError",
            format!("the riva SDK snapshot is invalid: {error}"),
        )
    })
}

pub(super) fn answer(
    probe: &Probe,
    logger: &RecordingLogger,
    outcome: Result<&TranscriptionReply, &RivaCallError>,
) -> Result<Value, Box<HelperError>> {
    match outcome {
        Ok(reply) => base(probe, logger, Value::Null, Value::Null, hidden_view(reply)),
        Err(error) => base(
            probe,
            logger,
            json!(error.status_code()),
            json!(error.to_string()),
            Value::Null,
        ),
    }
}

fn hidden_view(reply: &TranscriptionReply) -> Value {
    json!({
        "model": reply.model,
        "custom_llm_provider": reply.custom_llm_provider,
        "audio_transcription_duration": duration(&reply.duration),
    })
}

fn base(
    probe: &Probe,
    logger: &RecordingLogger,
    status: Value,
    message: Value,
    hidden: Value,
) -> Result<Value, Box<HelperError>> {
    let pre = logger.pre.as_ref().map(pre_view).transpose()?;
    let post = logger.post.as_ref().map(post_view).transpose()?;
    let response = logger
        .post
        .as_ref()
        .map(|record| response_view(&record.original_response))
        .transpose()?;
    Ok(json!({
        "status": status,
        "message": message,
        "events": probe.events,
        "auth": {"kwargs": probe.auth_kwargs, "args": probe.auth_args},
        "recognition": probe.recognition,
        "endpointing": {
            "kwargs": probe.endpointing_kwargs,
            "copied": probe.endpointing_copied,
        },
        "streaming": {
            "interim_results": probe.interim_results,
            "config_is_recognition": probe.config_is_recognition,
        },
        "chunk_sizes": probe.chunk_sizes.first(),
        "timeout_seconds": probe.timeout_seconds.first().copied().flatten(),
        "pre": pre,
        "post": post,
        "response": response,
        "hidden": hidden,
    }))
}

fn pre_view(record: &LoggedPre) -> Result<Value, Box<HelperError>> {
    let complete = json!({
        "recognition_config": encoded(&record.recognition_config)?,
        "nvcf_function_id_set": record.nvcf_function_id_set,
        "use_ssl": encoded(&record.use_ssl)?,
    });
    Ok(json!({
        "api_base": record.api_base,
        "atranscription": record.atranscription,
        "complete_input_dict": complete,
    }))
}

fn post_view(record: &LoggedPost) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "input": encoded(&record.input)?,
        "complete_input_dict": encoded(&record.recognition_config)?,
        "original_response": response_view(&record.original_response)?,
    }))
}

// the handler's response dict carries the response object's model fields beside its extras
fn response_view(original: &Json) -> Result<Value, Box<HelperError>> {
    let Json::Object(fields) = original else {
        return Err(helper_error(
            "JsonError",
            String::from("the transcription response is not an object"),
        ));
    };
    Ok(json!({
        "text": fields.get("text"),
        "task": fields.get("task"),
        "words": fields.get("words"),
        "duration": fields.get("duration"),
    }))
}

fn duration(stored: &StoredDuration) -> Option<f64> {
    match stored {
        StoredDuration::Absent => None,
        StoredDuration::Known(seconds) => seconds.as_ref().and_then(ToPrimitive::to_f64),
    }
}

fn encoded<T>(value: &T) -> Result<Value, Box<HelperError>>
where
    T: Serialize + ?Sized,
{
    match serde_json::to_value(value) {
        Ok(converted) => Ok(converted),
        Err(error) => Err(helper_error("JsonError", error.to_string())),
    }
}
