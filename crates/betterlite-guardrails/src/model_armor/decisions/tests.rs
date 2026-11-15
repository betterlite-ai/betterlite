#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::model_armor::decisions::wire::SanitizeResponse;

mod construction;
mod extraction;
mod redaction;
mod verdict;

fn response(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

fn scan(source: &str) -> SanitizeResponse {
    serde_json::from_str(source).unwrap()
}
