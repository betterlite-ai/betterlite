pub mod error;
pub mod model;

mod artifacts;
mod compare;
mod decode;
pub(crate) mod run;

mod tests;

use std::str;

use error::ScenarioError;
use model::{HttpScenario, Limits};

pub fn decode_http_scenario(bytes: &[u8], limits: &Limits) -> Result<HttpScenario, ScenarioError> {
    if bytes.len() > limits.max_input_bytes {
        return Err(ScenarioError::InputTooLarge {
            limit: limits.max_input_bytes,
        });
    }
    let text = str::from_utf8(bytes).map_err(ScenarioError::Utf8)?;
    decode::validate_root(text)?;
    let file: decode::ScenarioFile = serde_saphyr::from_str(text).map_err(ScenarioError::Parse)?;
    file.into_scenario(limits)
}
