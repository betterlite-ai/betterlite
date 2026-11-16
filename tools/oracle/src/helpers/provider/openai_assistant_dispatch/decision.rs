use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_assistant_dispatch::call::{
    DecisionsCall, DecisionsOperation, OpenAiAssistantOperation,
};
use betterlite_provider::openai_assistant_dispatch::reply::{DecisionsProjection, DecisionsReply};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::input::{DecisionsOp, Input};
use super::invalid_input;

// LiteLLM's globals stay unset, so the leaf resolves from the explicit values and the case's environment.
pub(super) fn decisions(
    input: &Input,
    operation: DecisionsOp,
) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    let request = input
        .decision
        .clone()
        .ok_or_else(|| invalid_input(String::from("decision is required")))?;
    let model = input.model.clone();
    let decision = match operation {
        DecisionsOp::Request => DecisionsOperation::Request { model, request },
        DecisionsOp::Response => DecisionsOperation::Response {
            model,
            request,
            payload: input.payload.clone().into_bytes(),
        },
    };
    Ok(OpenAiAssistantOperation::Decisions(DecisionsCall {
        api_base: input.api_base.clone(),
        api_key: input.api_key.clone(),
        globals: LiteLlmGlobals::default(),
        environment: input.environment.clone(),
        operation: decision,
    }))
}

pub(super) fn decision_view(reply: &DecisionsReply) -> Result<Value, Box<HelperError>> {
    let api_key = reply
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned());
    Ok(match &reply.projection {
        DecisionsProjection::Request(wire) => json!({
            "api_base": reply.api_base,
            "api_key": api_key,
            "request": serde_json::to_value(wire)?,
        }),
        DecisionsProjection::Response(response) => json!({
            "api_base": reply.api_base,
            "api_key": api_key,
            "response": serde_json::to_value(response)?,
        }),
    })
}
