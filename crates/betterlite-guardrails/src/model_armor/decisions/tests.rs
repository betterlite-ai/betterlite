#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

mod construction;
mod extraction;
mod redaction;
mod verdict;

fn response(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}
