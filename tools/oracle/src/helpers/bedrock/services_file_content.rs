use betterlite_bedrock::file_content::{FileContentError, RawResponse, transform_file_response};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    response: RawResponse,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let response = input.response.clone();
        let result = match transform_file_response(response) {
            Ok(content) => json!({
                "ok": true,
                "response": content.response,
                "hidden_params": content.hidden_params,
                "summary": content.logging_summary(),
            }),
            Err(FileContentError::Bedrock { body, error, .. }) => json!({
                "ok": false,
                "error": {
                    "status_code": error.cause.error.status_code,
                    "message": error.cause.error.message,
                    "headers": error.cause.response_headers,
                    "response": {
                        "status_code": error.cause.error.status_code,
                        "body": body,
                        "headers": error.cause.response_headers,
                        "request": error.cause.error.request,
                    },
                },
            }),
            Err(FileContentError::Header(error)) => {
                json!({"ok": false, "error": error.to_string()})
            }
        };
        Ok(result)
    }
}

mod tests;
