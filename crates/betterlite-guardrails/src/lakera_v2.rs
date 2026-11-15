use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub mod advisory;
pub mod error;
pub mod guardrail;
pub mod hooks;
pub mod scope;

pub type Data = BTreeMap<String, GuardrailValue>;

mod tests;
