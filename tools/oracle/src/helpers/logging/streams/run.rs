use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_logging::fields::usage::project::usage_json;
use betterlite_logging::logging_streams::cache_usage::backfill_cache_usage;
use betterlite_logging::logging_streams::overlay::overlay_served_output_texts;
use betterlite_logging::logging_streams::served_texts::{StreamTexts, served_output_texts};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;

use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{Input, Probe};

pub(crate) fn run(input: &Input) -> HelperResult {
    let mut output = Vec::with_capacity(input.probes.len());
    for probe in &input.probes {
        match probe {
            Probe::Chat { responses } => {
                let mut values = Vec::with_capacity(responses.len());
                for response in responses {
                    let response: Json =
                        serde_json::from_value(response.clone()).or_value_error()?;
                    values.push(
                        serde_json::to_value(served_output_texts(&response)).or_value_error()?,
                    );
                }
                output.push(Value::Array(values));
            }
            Probe::Stream { chunk_lists } => {
                let mut values = Vec::with_capacity(chunk_lists.len());
                for chunks in chunk_lists {
                    let mut texts = StreamTexts::default();
                    for chunk in chunks {
                        let chunk: Json = serde_json::from_value(chunk.clone()).or_value_error()?;
                        texts.push(&chunk);
                    }
                    values.push(serde_json::to_value(texts.finish()).or_value_error()?);
                }
                output.push(Value::Array(values));
            }
            Probe::PartialUsage { usages } => {
                let mut values = Vec::with_capacity(usages.len());
                for usage in usages {
                    let mapping: IndexMap<String, Json> =
                        serde_json::from_value(usage.clone()).or_value_error()?;
                    let mut parsed = usage_from_mapping(&mapping).or_value_error()?;
                    backfill_cache_usage(&mut parsed);
                    values.push(serde_json::to_value(usage_json(&parsed)).or_value_error()?);
                }
                output.push(Value::Array(values));
            }
            Probe::Overlay { response, texts } => {
                let response: Json = serde_json::from_value(response.clone()).or_value_error()?;
                let overlaid = overlay_served_output_texts(&response, texts.as_ref());
                output.push(serde_json::to_value(overlaid).or_value_error()?);
            }
        }
    }
    Ok(Value::Array(output))
}
