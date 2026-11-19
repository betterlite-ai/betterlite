use betterlite_bedrock::agentcore_search_reply::decode_agentcore_search_reply;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match decode_agentcore_search_reply(input.body.as_bytes(), input.status, &input.headers) {
            Ok(response) => Ok(serde_json::to_value(response)?),
            Err(error) => Err(Box::new(HelperError {
                class: "SearchReplyError".to_owned(),
                status: error.status(),
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })),
        }
    }
}
