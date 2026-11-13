use std::collections::BTreeMap;

use betterlite_policy::condition::evaluate_condition;
use betterlite_policy::context::PolicyMatchContext;
use betterlite_policy::inheritance::{resolve_inheritance_chain, resolve_policy_guardrails};
use betterlite_policy::pipeline::GuardrailPipeline;
use betterlite_policy::scope::{matches_pattern, scope_matches};
use betterlite_policy::values::{Policy, PolicyAttachment, PolicyCondition, PolicyScope};
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    function: String,
    condition: Option<serde_json::Value>,
    context: Option<serde_json::Value>,
    value: Option<String>,
    patterns: Option<Vec<String>>,
    scope: Option<serde_json::Value>,
    policy_name: Option<String>,
    policies: Option<BTreeMap<String, serde_json::Value>>,
    attachment: Option<serde_json::Value>,
    pipeline: Option<serde_json::Value>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function.as_str() {
            "evaluate_condition" => evaluate(input),
            "matches_pattern" => Ok(matches(input)),
            "scope_matches" => matched_scope(input),
            "inheritance_chain" => chain(input),
            "resolve_policy_guardrails" => resolved(input),
            "attachment_scope" => attachment_scope(input),
            "validate_pipeline" => validate_pipeline(input),
            _ => Err(invalid_input(format!(
                "unknown b3240 helper function {}",
                input.function
            ))),
        }
    }
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn decode<T>(value: serde_json::Value) -> Result<T, Box<HelperError>>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value)
        .map_err(|error| invalid_input(format!("invalid helper input: {error}")))
}

fn required(
    value: Option<&serde_json::Value>,
    name: &str,
) -> Result<serde_json::Value, Box<HelperError>> {
    value
        .cloned()
        .ok_or_else(|| invalid_input(format!("{name} must be an object")))
}

fn evaluate(input: &Input) -> HelperResult {
    let condition = input
        .condition
        .clone()
        .map(decode::<PolicyCondition>)
        .transpose()?;
    let context: PolicyMatchContext = decode(required(input.context.as_ref(), "context")?)?;
    let matched = evaluate_condition(condition.as_ref(), &context);
    Ok(serde_json::json!({ "matches": matched }))
}

fn matches(input: &Input) -> serde_json::Value {
    let patterns = input.patterns.clone().unwrap_or_default();
    let matched = matches_pattern(input.value.as_deref(), &patterns);
    serde_json::json!({ "matches": matched })
}

fn matched_scope(input: &Input) -> HelperResult {
    let scope: PolicyScope = decode(required(input.scope.as_ref(), "scope")?)?;
    let context: PolicyMatchContext = decode(required(input.context.as_ref(), "context")?)?;
    Ok(serde_json::json!({ "matches": scope_matches(&scope, &context) }))
}

fn policy_name(input: &Input) -> Result<String, Box<HelperError>> {
    input
        .policy_name
        .clone()
        .ok_or_else(|| invalid_input("policy_name must be a string".to_owned()))
}

fn policies(input: &Input) -> Result<BTreeMap<String, Policy>, Box<HelperError>> {
    let entries = input
        .policies
        .clone()
        .ok_or_else(|| invalid_input("policies must be an object".to_owned()))?;
    entries
        .into_iter()
        .map(|(name, entry)| {
            serde_json::from_value::<Policy>(entry)
                .map(|policy| (name, policy))
                .map_err(|error| invalid_input(error.to_string()))
        })
        .collect()
}

fn chain(input: &Input) -> HelperResult {
    let name = policy_name(input)?;
    let entries = policies(input)?;
    let chain = resolve_inheritance_chain(&name, &entries);
    Ok(serde_json::json!({ "chain": chain }))
}

fn resolved(input: &Input) -> HelperResult {
    let policies = policies(input)?;
    let context = input
        .context
        .clone()
        .map(decode::<PolicyMatchContext>)
        .transpose()?;
    let resolved = resolve_policy_guardrails(&policy_name(input)?, &policies, context.as_ref());
    Ok(serde_json::json!({
        "guardrails": resolved.guardrails,
        "chain": resolved.inheritance_chain,
    }))
}

fn attachment_scope(input: &Input) -> HelperResult {
    let attachment: PolicyAttachment = decode(required(input.attachment.as_ref(), "attachment")?)?;
    let scope = attachment.to_policy_scope();
    Ok(serde_json::json!({
        "teams": scope.get_teams(),
        "keys": scope.get_keys(),
        "models": scope.get_models(),
        "tags": scope.get_tags(),
    }))
}

fn validate_pipeline(input: &Input) -> HelperResult {
    decode::<GuardrailPipeline>(required(input.pipeline.as_ref(), "pipeline")?)
        .map(|pipeline| serde_json::json!({ "valid": true, "steps": pipeline.steps.len() }))
}
