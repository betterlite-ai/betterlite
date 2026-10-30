use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    UnwrapVerdict { response: Value },
    HasDecisionFields { response: Value },
    NormalizeAction { action: String },
    ExtractJsonrpcError { response: Value },
    ExtractSanitizedText { response: Value },
    ExtractSanitizedMessages { response: Value },
    ExtractSanitizedMcpArguments { response: Value },
    ExtractMaskedEntityCount { rules: Value },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::UnwrapVerdict { response } => {
                betterlite_guardrails::cisco_verdict::unwrap_verdict_envelope(response)
            }
            Input::HasDecisionFields { response } => json!(
                betterlite_guardrails::cisco_verdict::has_decision_fields(response)
            ),
            Input::NormalizeAction { action } => json!(
                betterlite_guardrails::cisco_verdict::normalize_action(action)
            ),
            Input::ExtractJsonrpcError { response } => {
                betterlite_guardrails::cisco_verdict::extract_jsonrpc_error(response)
                    .unwrap_or(Value::Null)
            }
            Input::ExtractSanitizedText { response } => {
                betterlite_guardrails::cisco_verdict::extract_sanitized_text(response)
                    .map_or(Value::Null, Value::String)
            }
            Input::ExtractSanitizedMessages { response } => {
                betterlite_guardrails::cisco_verdict::extract_sanitized_messages(response)
                    .map_or(Value::Null, |messages| json!(messages))
            }
            Input::ExtractSanitizedMcpArguments { response } => {
                betterlite_guardrails::cisco_verdict::extract_mcp_args(response)
                    .map_or(Value::Null, |arguments| json!(arguments))
            }
            Input::ExtractMaskedEntityCount { rules } => {
                betterlite_guardrails::cisco_verdict::extract_masked_entity_count(rules)
                    .map_or(Value::Null, |counts| json!(counts))
            }
        };
        Ok(value)
    }
}
