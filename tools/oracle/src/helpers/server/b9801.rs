use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::messages_first_chunk::frame_error::{frame_error_code, frame_error_object};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    text: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let bytes = input.text.as_bytes();
        Ok(json!({
            "code": frame_error_code(bytes),
            "error": frame_error_object(bytes),
        }))
    }
}
