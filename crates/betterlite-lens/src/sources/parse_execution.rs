use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseExecutionError {
    #[error("the execution id is not valid base64")]
    Base64(#[source] base64::DecodeError),
    #[error("the execution id is not a JSON array of strings")]
    Json(#[source] serde_json::Error),
    #[error("the execution id has {0} elements, expected 3 or 4")]
    ElementCount(usize),
}

pub fn parse_execution(
    value: &str,
) -> Result<(String, String, String, String), ParseExecutionError> {
    let bytes = URL_SAFE
        .decode(value)
        .map_err(ParseExecutionError::Base64)?;
    let elements: Vec<String> =
        serde_json::from_slice(&bytes).map_err(ParseExecutionError::Json)?;
    match <[String; 3]>::try_from(elements.clone()) {
        Ok([source, team_id, trace_id]) => Ok((source, team_id, trace_id, String::new())),
        Err(_) => match <[String; 4]>::try_from(elements) {
            Ok(four) => Ok(<(String, String, String, String)>::from(four)),
            Err(other) => Err(ParseExecutionError::ElementCount(other.len())),
        },
    }
}

mod tests;
