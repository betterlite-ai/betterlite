use std::collections::BTreeMap;

use betterlite_openai::cohere_streams::{CohereStreamError, CohereStreamVersion};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value};

mod tests;

#[derive(Deserialize)]
pub(crate) struct Input {
    version: CohereStreamVersion,
    record: String,
    #[serde(default)]
    bytes: bool,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: SecretString,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut headers = input.headers.clone();
        betterlite_openai::cohere_streams::validate_environment(&mut headers, &input.api_key);
        let decoded = betterlite_openai::cohere_streams::decode_cohere_line(
            input.version,
            input.record.as_bytes(),
            input.bytes,
        );
        let decoded_chunk = decoded.map_err(|error| error_result(&error))?;
        let chunk_value = serde_json::to_value(decoded_chunk)
            .map_err(|source| helper_error("InternalError", source.to_string()))?;
        Ok(Value::Object(Map::from_iter([
            (
                String::from("headers"),
                serde_json::to_value(headers)
                    .map_err(|source| helper_error("InternalError", source.to_string()))?,
            ),
            (String::from("chunk"), chunk_value),
        ])))
    }
}

fn error_result(error: &CohereStreamError) -> Box<crate::helpers::HelperError> {
    let class = match error {
        CohereStreamError::Utf8(_) => "UnicodeDecodeError",
        CohereStreamError::Json(source) => {
            if source.is_data() {
                "TypeError"
            } else {
                "JSONDecodeError"
            }
        }
        CohereStreamError::MissingFinishReason => "KeyError",
        CohereStreamError::NumericIndex(_)
        | CohereStreamError::IndexSeparator
        | CohereStreamError::InvalidIndex { .. }
        | CohereStreamError::V2Transform { .. }
        | CohereStreamError::UsageOverflow
        | CohereStreamError::TooLarge => "ValueError",
    };
    helper_error(class, error.to_string())
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from(class),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
