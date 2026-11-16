use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_logging::fields::usage::project::usage_json;
use betterlite_logging::logging_streams::cache_usage::backfill_cache_usage;
use betterlite_logging::logging_streams::overlay::overlay_served_output_texts;
use betterlite_logging::logging_streams::served_texts::{StreamTexts, served_output_texts};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{Input, Probe};

pub(crate) fn run(input: &Input) -> HelperResult {
    let mut output = Vec::with_capacity(input.probes.len());
    for probe in &input.probes {
        let value = match probe {
            Probe::Chat { responses } => chat_value(responses)?,
            Probe::Stream { chunk_lists } => stream_value(chunk_lists)?,
            Probe::PartialUsage { usages } => partial_usage_value(usages)?,
            Probe::Overlay { response, texts } => overlay_value(response, texts.as_ref())?,
        };
        output.push(value);
    }
    Ok(Value::Array(output))
}

fn chat_value(responses: &[Value]) -> HelperResult {
    let mut values = Vec::with_capacity(responses.len());
    for raw in responses {
        let response: Json = serde_json::from_value(raw.clone()).or_value_error()?;
        values.push(serde_json::to_value(served_output_texts(&response)).or_value_error()?);
    }
    Ok(Value::Array(values))
}

fn stream_value(chunk_lists: &[Vec<Value>]) -> HelperResult {
    let mut values = Vec::with_capacity(chunk_lists.len());
    for chunks in chunk_lists {
        let mut texts = StreamTexts::default();
        for raw in chunks {
            let chunk: Json = serde_json::from_value(raw.clone()).or_value_error()?;
            texts.push(&chunk);
        }
        values.push(serde_json::to_value(texts.finish()).or_value_error()?);
    }
    Ok(Value::Array(values))
}

fn partial_usage_value(usages: &[Value]) -> HelperResult {
    let mut values = Vec::with_capacity(usages.len());
    for usage in usages {
        let mapping: IndexMap<String, Json> =
            serde_json::from_value(usage.clone()).or_value_error()?;
        let mut parsed = usage_from_mapping(&mapping).or_value_error()?;
        backfill_cache_usage(&mut parsed);
        values.push(serde_json::to_value(usage_json(&parsed)).or_value_error()?);
    }
    Ok(Value::Array(values))
}

fn overlay_value(response: &Value, texts: Option<&Vec<Option<String>>>) -> HelperResult {
    let logged: Json = serde_json::from_value(response.clone()).or_value_error()?;
    let overlaid = overlay_served_output_texts(&logged, texts);
    serde_json::to_value(overlaid).or_value_error()
}
