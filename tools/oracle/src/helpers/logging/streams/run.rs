use betterlite_logging::logging_streams::overlay::overlay_served_output_texts;
use betterlite_logging::logging_streams::served_texts::{
    served_output_texts, served_stream_output_texts,
};
use betterlite_models::decisions::Json;
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
                    let mut decoded = Vec::with_capacity(chunks.len());
                    for chunk in chunks {
                        decoded
                            .push(serde_json::from_value::<Json>(chunk.clone()).or_value_error()?);
                    }
                    values.push(
                        serde_json::to_value(served_stream_output_texts(&decoded))
                            .or_value_error()?,
                    );
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
