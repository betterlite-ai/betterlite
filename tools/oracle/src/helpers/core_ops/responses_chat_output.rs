use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_chat_output::assemble_response;
use betterlite_core::api::responses_chat_output::records::{ChatCompletionResponse, RequestFields};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatOutputInput {
    request_input: Json,
    responses_api_request: RequestFields,
    chat_completion_response: ChatCompletionResponse,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = ChatOutputInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut ids = 0_u32;
        let assembled = assemble_response(
            &input.request_input,
            &input.responses_api_request,
            &input.chat_completion_response,
            ApiLimits::default(),
            || {
                ids = ids.saturating_add(1);
                format!("oracle-{ids}")
            },
        )
        .or_value_error()?;
        Ok(json!(assembled.response))
    }
}
