use std::collections::BTreeMap;

use betterlite_openai::cohere_streams::{CohereStreamError, CohereStreamVersion};
use serde::Deserialize;
use serde_json::{Map, Value};

mod tests;

#[derive(Deserialize)]
pub(crate) struct Input {
    version: String,
    record: String,
    #[serde(default)]
    bytes: bool,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: String,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let version = parse_version(&input.version)?;
        let mut headers = input.headers.clone();
        betterlite_openai::cohere_streams::validate_environment(&mut headers, &input.api_key);
        let decoded = betterlite_openai::cohere_streams::decode_cohere_line(
            version,
            input.record.as_bytes(),
            input.bytes,
        );
        let decoded_chunk = decoded.map_err(|error| error_result(&error, input))?;
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

fn parse_version(version: &str) -> Result<CohereStreamVersion, Box<crate::helpers::HelperError>> {
    match version {
        "v1" => Ok(CohereStreamVersion::V1),
        "v2" => Ok(CohereStreamVersion::V2),
        _ => Err(helper_error(
            "InvalidInput",
            String::from("unknown Cohere version"),
        )),
    }
}

fn error_result(error: &CohereStreamError, input: &Input) -> Box<crate::helpers::HelperError> {
    let class = error_class(error);
    let message = if class == "JSONDecodeError" && input.bytes && input.record.contains("data:") {
        String::from("Expecting value: line 1 column 1 (char 0)")
    } else {
        error.to_string()
    };
    helper_error(class, message)
}

const fn error_class(error: &CohereStreamError) -> &'static str {
    match error {
        CohereStreamError::Utf8(_) | CohereStreamError::ReceivedUtf8 { .. } => "UnicodeDecodeError",
        CohereStreamError::Json(_) => "JSONDecodeError",
        CohereStreamError::Index
        | CohereStreamError::InvalidIndex { .. }
        | CohereStreamError::V2Transform(_) => "ValueError",
    }
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
