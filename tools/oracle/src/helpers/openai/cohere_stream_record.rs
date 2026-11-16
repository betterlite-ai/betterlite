use std::collections::BTreeMap;

use betterlite_openai::cohere_streams;
use betterlite_openai::cohere_streams::{CohereChunk, CohereStreamError, CohereStreamVersion};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

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

#[derive(Serialize)]
struct Output {
    headers: BTreeMap<String, String>,
    chunk: CohereChunk,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut headers = input.headers.clone();
        cohere_streams::validate_environment(&mut headers, &input.api_key);
        let decoded =
            cohere_streams::decode_cohere_line(input.version, input.record.as_bytes(), input.bytes);
        let chunk = decoded.map_err(|error| error_result(&error))?;
        serde_json::to_value(Output { headers, chunk }).map_err(|source| {
            Box::new(HelperError {
                class: String::from("InternalError"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

fn error_result(error: &CohereStreamError) -> Box<HelperError> {
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
    Box::new(HelperError {
        class: String::from(class),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
