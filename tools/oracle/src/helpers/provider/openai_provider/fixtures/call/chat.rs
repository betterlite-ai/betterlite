use betterlite_openai::chat::ChatRequest;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_dispatch::call::ChatCall;
use serde_json::Value;

use super::super::case::{Case, failure};
use super::{encoded, explicit_credentials, metadata};

pub(super) fn chat_call(case: &Case) -> Result<ChatCall, Box<HelperError>> {
    let mut body = case.optional_params.clone();
    body.insert(
        String::from("messages"),
        Value::Array(case.messages.clone()),
    );
    let request =
        ChatRequest::parse(&encoded(body)?).map_err(|error| failure("ValueError", error))?;
    Ok(ChatCall {
        model: case.model.clone(),
        request,
        credentials: explicit_credentials(case),
        headers: case.headers.clone(),
        extra_headers: case.extra_headers.clone(),
        timeout: None,
        metadata: metadata(case.metadata.as_ref())?,
    })
}
