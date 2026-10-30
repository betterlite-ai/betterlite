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
        let result = match transform_file_response(input.response.clone()) {
            Ok(content) => json!({
                "ok": true,
                "response": content.response,
                "hidden_params": content.hidden_params,
                "summary": content.logging_summary(),
            }),
            Err(FileContentError::Bedrock(error)) => {
                json!({"ok": false, "error": error})
            }
        };
        Ok(result)
    }
}

mod tests;
