use std::collections::BTreeMap;

use betterlite_config::api::responses_stream_primitives::{
    STREAM_DURATION_ENV, StreamDurationError, loads_stream_duration_settings,
};
use betterlite_logging::analytics::python::float_text;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::base::{ProbeSpec, build_state};
use super::support::MOMENT;

#[derive(Debug, Deserialize)]
pub(crate) struct DurationRow {
    #[serde(default)]
    environment: Option<String>,
    created: f64,
    now: f64,
    model: String,
    #[serde(default)]
    provider: Option<String>,
}

pub(crate) fn answer(rows: &[DurationRow]) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        answers.push(row_answer(row)?);
    }
    Ok(json!({ "rows": answers }))
}

fn row_answer(row: &DurationRow) -> Result<Value, Box<HelperError>> {
    let Ok(limit) = settings(row.environment.as_deref()) else {
        return Ok(json!({ "loaded": false }));
    };
    let spec = ProbeSpec {
        model: row.model.clone(),
        provider: row.provider.clone(),
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        start_time: MOMENT.to_owned(),
        created: row.created,
        litellm_params: IndexMap::new(),
        headers: IndexMap::new(),
    };
    let state = build_state(&spec, limit)?;
    let checked = state.check_max_streaming_duration(row.now);
    let (raised, status, model, provider) = match checked {
        Ok(()) => (
            false,
            None,
            state.model,
            state.custom_llm_provider.unwrap_or_default(),
        ),
        Err(timeout) => (
            true,
            Some(timeout.status_code),
            timeout.model,
            timeout.custom_llm_provider,
        ),
    };
    Ok(json!({
        "loaded": true,
        "limit": limit.map(float_text),
        "raised": raised,
        "status": status,
        "model": model,
        "llm_provider": provider,
    }))
}

fn settings(environment: Option<&str>) -> Result<Option<f64>, StreamDurationError> {
    let mut values = BTreeMap::new();
    if let Some(text) = environment {
        values.insert(STREAM_DURATION_ENV.to_owned(), text.to_owned());
    }
    loads_stream_duration_settings(&values).map(|settings| settings.max_streaming_duration_seconds)
}
