use std::path::Path;

use betterlite_core::api::riva::audio::ResampledAudio;
use betterlite_core::api::riva::audio::error::RivaAudioError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_pyhost::host::PythonHost;
use pyo3::types::PyAnyMethods as _;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Value, json};

use super::helper_error;

pub(super) struct Probe {
    events: Vec<String>,
    staging: Vec<String>,
}

struct StagingState<'staging> {
    suffix: Option<&'staging str>,
    removed: Option<bool>,
}

pub(super) fn read_probe(host: &PythonHost) -> Result<Probe, Box<HelperError>> {
    host.attach(|py| {
        let module = py.import("riva_audio_probe")?;
        let events: Vec<String> = module.getattr("events")?.extract()?;
        let staging: Vec<String> = module.getattr("staging")?.extract()?;
        Ok(Probe { events, staging })
    })
    .map_err(|error| helper_error("PythonError", error.to_string()))
}

pub(super) fn success_answer(audio: &ResampledAudio, probe: Probe) -> Value {
    let Probe { events, staging } = probe;
    let state = staging_state(&staging);
    json!({
        "status": Value::Null,
        "message": Value::Null,
        "pcm_hex": hex::encode(audio.pcm_bytes.as_slice()),
        "sample_rate_hz": audio.sample_rate_hz,
        "num_channels": audio.num_channels,
        "duration_seconds": audio.duration_seconds.to_f64(),
        "events": events,
        "staging_suffix": state.suffix,
        "staging_removed": state.removed,
    })
}

pub(super) fn failure_answer(error: &RivaAudioError, probe: Probe) -> Value {
    let Probe { events, staging } = probe;
    let state = staging_state(&staging);
    let response = error.response();
    json!({
        "status": response.status_code,
        "message": error.to_string(),
        "pcm_hex": Value::Null,
        "sample_rate_hz": Value::Null,
        "num_channels": Value::Null,
        "duration_seconds": Value::Null,
        "events": events,
        "staging_suffix": state.suffix,
        "staging_removed": state.removed,
    })
}

fn staging_state(staging: &[String]) -> StagingState<'_> {
    let Some(path) = staging.last() else {
        return StagingState {
            suffix: None,
            removed: None,
        };
    };
    let suffix = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str());
    StagingState {
        suffix,
        removed: Some(!Path::new(path).exists()),
    }
}
