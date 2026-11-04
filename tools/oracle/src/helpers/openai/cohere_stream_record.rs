use serde::Deserialize;

mod tests;

#[derive(Deserialize)]
pub(crate) struct Input {
    version: String,
    record: String,
    #[serde(default)]
    bytes: bool,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    api_key: String,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let version = match input.version.as_str() {
            "v1" => betterlite_openai::cohere_streams::CohereStreamVersion::V1,
            "v2" => betterlite_openai::cohere_streams::CohereStreamVersion::V2,
            _ => {
                return Err(Box::new(crate::helpers::HelperError {
                    class: String::from("InvalidInput"),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: String::from("unknown Cohere version"),
                }));
            }
        };
        let mut headers = input.headers.clone();
        betterlite_openai::cohere_streams::validate_environment(&mut headers, &input.api_key);
        let decoded = betterlite_openai::cohere_streams::decode_cohere_line(
            version,
            input.record.as_bytes(),
            input.bytes,
        );
        let result = match decoded {
            Ok(chunk) => serde_json::to_value(chunk).map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: String::from("InternalError"),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?,
            Err(error) => {
                let class = match &error {
                    betterlite_openai::cohere_streams::CohereStreamError::Utf8(_) => {
                        "UnicodeDecodeError"
                    }
                    betterlite_openai::cohere_streams::CohereStreamError::Json(_) => {
                        "JSONDecodeError"
                    }
                    betterlite_openai::cohere_streams::CohereStreamError::Index
                    | betterlite_openai::cohere_streams::CohereStreamError::V2Transform(_) => {
                        "ValueError"
                    }
                };
                let message = if class == "JSONDecodeError"
                    && input.bytes
                    && input.record.contains("data:")
                {
                    String::from("Expecting value: line 1 column 1 (char 0)")
                } else {
                    error.to_string()
                };
                return Err(Box::new(crate::helpers::HelperError {
                    class: String::from(class),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message,
                }));
            }
        };
        Ok(serde_json::json!({"headers": headers, "chunk": result}))
    }
}
