use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_openai::perplexity_chat::provider_info;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Steps {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    api_base: Option<String>,
    api_key: Option<String>,
    environment: BTreeMap<String, String>,
}

pub(super) fn run(steps: &Steps) -> Value {
    let mut outcomes = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        outcomes.push(outcome(step));
    }
    Value::Array(outcomes)
}

fn outcome(step: &Step) -> Value {
    let key = step
        .api_key
        .as_ref()
        .map(|text| SecretString::from(text.clone()));
    let (base, resolved) = provider_info(step.api_base.as_deref(), key.as_ref(), &step.environment);
    json!({
        "base": base,
        "key": resolved.as_ref().map(|secret| secret.expose_secret().to_owned()),
    })
}
