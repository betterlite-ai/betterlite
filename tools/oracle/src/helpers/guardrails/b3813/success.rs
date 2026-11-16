use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::tool_permission_responses::success_hook::{
    ResponseFormat, post_call_success_hook,
};
use serde_json::Value;

use betterlite_oracle_helper_api::helper::HelperResult;

use super::support::{build_guardrail, failure};
use super::views::{applied_guardrails, blocked_view, response_view};
use super::{Format, Input};

pub(crate) fn success(input: &Input) -> HelperResult {
    let guardrail = build_guardrail(input)?;
    let format = match input.format {
        Format::Openai => ResponseFormat::OpenAi,
        Format::Anthropic => ResponseFormat::Anthropic,
    };
    let Some(response_fields) = &input.response else {
        return Err(failure(
            "ValueError",
            "success mode needs a response".to_owned(),
        ));
    };
    let mut data = input.data.clone();
    let mut response = GuardrailValue::Object(response_fields.clone());
    let blocked = match post_call_success_hook(
        &guardrail,
        &mut data,
        format,
        &BTreeSet::new(),
        &mut response,
    ) {
        Ok(()) => Value::Null,
        Err(error) => match blocked_view(&error) {
            Some(blocked) => blocked,
            None => return Err(failure("GuardrailFailure", error.to_string())),
        },
    };
    let projected = response_view(input.format, &response)?;
    Ok(serde_json::json!({
        "blocked": blocked,
        "response": projected,
        "applied_guardrails": applied_guardrails(&data),
    }))
}
