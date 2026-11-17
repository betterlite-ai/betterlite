use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_dispatch::call::TextCall;
use serde_json::Value;

use super::super::case::{Case, failure};
use super::{explicit_credentials, merged_headers};

pub(super) fn text_call(case: &Case) -> Result<TextCall, Box<HelperError>> {
    let messages = serde_json::from_value(Value::Array(case.messages.clone()))
        .map_err(|error| failure("ValueError", error))?;
    Ok(TextCall {
        model: case.model.clone(),
        messages,
        optional_params: case.optional_params.clone(),
        credentials: explicit_credentials(case),
        headers: merged_headers(case),
        timeout: None,
        acompletion: false,
        text_completion: case.text_completion,
    })
}
