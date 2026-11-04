use std::collections::BTreeMap;

use betterlite_openai::cohere_streams::{CohereChunk, CohereStreamError, CohereStreamVersion};
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
        let chunk = decoded.map_err(|error| error_result(&error))?;
        serde_json::to_value(Output { headers, chunk }).map_err(|source| {
            Box::new(crate::helpers::HelperError {
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
    Box::new(crate::helpers::HelperError {
        class: String::from(class),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
