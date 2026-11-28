use std::error::Error as StdError;

use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::bedrock_other_dispatch::test_support::record::Recorded;
use serde_json::{Value, json};

use super::harness::Outcome;
use super::model::{Case, Failed, Input, Probe};
use crate::openai_media_dispatch::failure;

mod count;
mod embedding;
mod image;
mod rerank;
mod status;
mod transcription;

// Every probe runs the same production dispatch entry with its own fake transport
pub(super) async fn probe_rows(input: &Input) -> HelperResult {
    let mut rows = Vec::with_capacity(input.probes.len());
    for probe in &input.probes {
        rows.push(probe_row(input, probe).await?);
    }
    Ok(Value::Array(rows))
}

async fn probe_row(input: &Input, probe: &Probe) -> Result<Value, Failed> {
    match probe.case {
        Case::Embedding => embedding::embedding_row(probe).await,
        Case::EmbeddingStatus => status::status_row(probe).await,
        Case::Rerank => rerank::rerank_row(probe).await,
        Case::Count => count::count_row(probe).await,
        Case::Transcription => transcription::transcription_row(probe).await,
        Case::ImageGeneration => image::image_row(input, probe, false).await,
        Case::ImageEdit => image::image_row(input, probe, true).await,
    }
}

// One row per probe: the posted requests, whether the dispatch succeeded, and what it returned
pub(super) fn row<E>(
    outcome: &Outcome<E>,
    value: Value,
    error: Option<&str>,
) -> Result<Value, Failed>
where
    E: StdError + 'static,
{
    let requests = request_list(&outcome.requests)?;
    let status = match &outcome.result {
        Ok(_ok) => None,
        Err(reported) => reported.status(),
    };
    Ok(json!({
        "requests": requests,
        "ok": outcome.result.is_ok(),
        "status": status,
        "error": error,
        "value": value,
    }))
}

fn request_list(requests: &[Recorded]) -> Result<Value, Failed> {
    let mut values = Vec::with_capacity(requests.len());
    for request in requests {
        values.push(request_value(request)?);
    }
    Ok(Value::Array(values))
}

fn request_value(request: &Recorded) -> Result<Value, Failed> {
    let body = if request.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&request.body).map_err(|error| failure("Request", error))?
    };
    Ok(json!({
        "url": &request.url,
        "body": body,
        "authorization": authorization(request),
    }))
}

fn authorization(request: &Recorded) -> Value {
    match header(request, "authorization") {
        None => Value::Null,
        Some(value) if value.starts_with("AWS4-HMAC-SHA256") => Value::String("sigv4".to_owned()),
        Some(_other) => Value::String("other".to_owned()),
    }
}

fn header<'request>(request: &'request Recorded, name: &str) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(key, _value)| key.eq_ignore_ascii_case(name))
        .map(|(_key, value)| value.as_str())
}
