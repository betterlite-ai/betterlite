use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};

pub mod error;

use error::ConductError;

pub const BLOCKING_VERDICTS: [&str; 2] = ["block", "approval"];
pub const FLAGGED_VERDICTS: [&str; 2] = ["warning", "advisory"];
pub const UNREACHABLE_FALLBACK_PARAMETER: &str = "unreachable_fallback";
const DEFAULT_ROLE: &str = "user";
const PROMPT_KEY: &str = "prompt";
const MESSAGES_KEY: &str = "messages";
const ROLE_KEY: &str = "role";
const CONTENT_KEY: &str = "content";
const VERDICT_KEY: &str = "verdict";
const RULE_ID_KEY: &str = "rule_id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConductDecision {
    pub verdict: String,
    pub rule_id: Option<String>,
}

pub trait ConductCheck {
    fn call(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        call_type: &str,
    ) -> impl Future<Output = Result<ConductDecision, ConductError>> + Send;
}

pub struct ConductEnforcement<'state, R> {
    pub request_data: &'state mut BTreeMap<String, GuardrailValue>,
    pub blocked: fn(&ConductDecision) -> GuardrailFailure,
    pub record: R,
}

pub fn request_payload(
    inputs: &GuardrailApiInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Option<BTreeMap<String, GuardrailValue>> {
    if input_type != GuardrailInputType::Request {
        return None;
    }
    let structured = inputs.structured_messages.as_deref().unwrap_or_default();
    let messages = if structured.is_empty() {
        user_messages(inputs.texts.as_deref().unwrap_or_default())
    } else {
        structured.iter().map(message_value).collect()
    };
    let mut payload = request_data.clone();
    payload.insert(PROMPT_KEY.to_owned(), GuardrailValue::Null);
    payload.insert(MESSAGES_KEY.to_owned(), GuardrailValue::Array(messages));
    Some(payload)
}

fn user_messages(texts: &[String]) -> Vec<GuardrailValue> {
    texts
        .iter()
        .map(|text| {
            GuardrailValue::Object(BTreeMap::from([
                (
                    ROLE_KEY.to_owned(),
                    GuardrailValue::Text(DEFAULT_ROLE.to_owned()),
                ),
                (CONTENT_KEY.to_owned(), GuardrailValue::Text(text.clone())),
            ]))
        })
        .collect()
}

fn message_value(message: &GuardrailMessage) -> GuardrailValue {
    let mut object = message.extra.clone();
    object.insert(
        ROLE_KEY.to_owned(),
        GuardrailValue::Text(message.role.clone()),
    );
    GuardrailValue::Object(object)
}

#[must_use]
pub fn decision_status(decision: &ConductDecision) -> GuardrailStatus {
    if FLAGGED_VERDICTS.contains(&decision.verdict.as_str()) {
        GuardrailStatus::GuardrailFlagged
    } else {
        GuardrailStatus::Success
    }
}

pub fn record_decision(
    context: &mut GuardrailRecordContext<'_>,
    decision: &ConductDecision,
) -> Result<(), GuardrailRecordError> {
    let rule_id = match &decision.rule_id {
        Some(rule_id) => GuardrailValue::Text(rule_id.clone()),
        None => GuardrailValue::Null,
    };
    let response = GuardrailValue::Object(BTreeMap::from([
        (
            VERDICT_KEY.to_owned(),
            GuardrailValue::Text(decision.verdict.clone()),
        ),
        (RULE_ID_KEY.to_owned(), rule_id),
    ]));
    add_guardrail_record(
        context,
        GuardrailRecordInput {
            guardrail_response: response,
            guardrail_status: decision_status(decision),
            start_time: None,
            end_time: None,
            duration: None,
            masked_entity_count: None,
            guardrail_provider: None,
            event_type: None,
            tracing_detail: None,
        },
    )
}

pub async fn apply_conduct_guardrail<C, R>(
    inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
    check: &C,
    enforcement: &mut ConductEnforcement<'_, R>,
) -> Result<GuardrailApiInputs, ConductError>
where
    C: ConductCheck,
    R: FnMut(&ConductDecision) -> Result<(), GuardrailRecordError>,
{
    let Some(payload) = request_payload(&inputs, enforcement.request_data, input_type) else {
        return Ok(inputs);
    };
    let decision = check.call(&payload, call_type(input_type)).await?;
    if BLOCKING_VERDICTS.contains(&decision.verdict.as_str()) {
        return Err(ConductError::Blocked((enforcement.blocked)(&decision)));
    }
    (enforcement.record)(&decision)?;
    Ok(inputs)
}

const fn call_type(input_type: GuardrailInputType) -> &'static str {
    match input_type {
        GuardrailInputType::Request => "request",
        GuardrailInputType::Response => "response",
    }
}

#[must_use]
pub fn binds_unreachable_fallback(parameters: &[String]) -> bool {
    for parameter in parameters {
        if parameter == UNREACHABLE_FALLBACK_PARAMETER {
            return true;
        }
    }
    false
}

mod tests;
