use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::wire::{DynamoAiResponse, FinalAction, PolicyOutputs};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuardrailStatus {
    #[default]
    Success,
    Intervened,
    FailedToRespond,
}

impl GuardrailStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Intervened => "guardrail_intervened",
            Self::FailedToRespond => "guardrail_failed_to_respond",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessedResult {
    pub violations_detected: Vec<String>,
    pub violation_details: BTreeMap<String, ViolationDetails>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViolationDetails {
    pub policy_id: Option<String>,
    pub action: String,
    pub method: Option<String>,
    pub description: Option<String>,
    pub message: Option<String>,
}

#[must_use]
pub const fn determine_status(response: &DynamoAiResponse) -> GuardrailStatus {
    if response.error.is_some() {
        return GuardrailStatus::FailedToRespond;
    }
    match response.final_action {
        FinalAction::None | FinalAction::Warn | FinalAction::Redact | FinalAction::Sanitize => {
            GuardrailStatus::Success
        }
        FinalAction::Block => GuardrailStatus::Intervened,
    }
}

// only a BLOCK final action reports violations, and inside it only policies whose own action is BLOCK
#[must_use]
pub fn process_response(response: &DynamoAiResponse) -> ProcessedResult {
    let mut processed = ProcessedResult::default();
    if response.final_action != FinalAction::Block {
        return processed;
    }
    for applied in &response.applied_policies {
        let outputs = applied.outputs.as_ref();
        let action = policy_action(applied.action.as_deref(), outputs);
        if action != "BLOCK" {
            continue;
        }
        let name = applied
            .policy
            .name
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        processed.violations_detected.push(name.clone());
        let details = ViolationDetails {
            policy_id: applied.policy.id.clone(),
            action,
            method: applied.policy.method.clone(),
            description: applied.policy.description.clone(),
            message: outputs.and_then(|found| found.message.clone()),
        };
        processed.violation_details.insert(name, details);
    }
    processed
}

fn policy_action(outer: Option<&str>, outputs: Option<&PolicyOutputs>) -> String {
    if let Some(action) = outer {
        return action.to_owned();
    }
    outputs
        .and_then(|found| found.action)
        .unwrap_or(FinalAction::None)
        .as_str()
        .to_owned()
}

#[must_use]
pub fn create_error_message(processed: &ProcessedResult) -> String {
    let mut message = format!(
        "the DynamoAI guardrail blocked the request: {} violation(s) detected\n\n",
        processed.violations_detected.len()
    );
    for policy_name in &processed.violations_detected {
        let _written = writeln!(message, "- {}:", policy_name.to_uppercase());
        if let Some(details) = processed.violation_details.get(policy_name) {
            push_detail(&mut message, "Action", Some(details.action.as_str()));
            push_detail(&mut message, "Method", details.method.as_deref());
            push_detail(&mut message, "Description", details.description.as_deref());
            push_detail(&mut message, "Message", details.message.as_deref());
            push_detail(&mut message, "Policy ID", details.policy_id.as_deref());
        }
        message.push('\n');
    }
    message.trim().to_owned()
}

fn push_detail(message: &mut String, label: &str, value: Option<&str>) {
    if let Some(text) = value.filter(|candidate| !candidate.is_empty()) {
        let _written = writeln!(message, "  {label}: {text}");
    }
}
