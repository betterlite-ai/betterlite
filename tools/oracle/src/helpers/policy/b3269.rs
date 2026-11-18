use std::collections::BTreeMap;

use betterlite_core::api::response_id_codecs::response::get_model_id_from_response_id;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let function = input
            .get("function")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_input("function must be a string"))?;
        match function {
            "model_group_lookup" => model_group_lookup(input),
            "submit_model_hidden_by" => submit_model_hidden_by(input),
            "retrieval_context" => retrieval_context(input),
            "pipeline_filtering" | "deduplication" => pipeline_filtering(input),
            "registry_not_initialized" => Ok(json!({"attached": false})),
            _ => Err(invalid_input(format!(
                "unknown b3269 helper function {function}"
            ))),
        }
    }
}

fn model_group_lookup(input: &Value) -> HelperResult {
    if input.get("router") == Some(&Value::Bool(false)) {
        return Ok(json!({"model_group": null, "reason": "no router", "hidden_by": null}));
    }
    let response_id = input.get("response_id").and_then(Value::as_str);
    let Some(model_id) = get_model_id_from_response_id(response_id) else {
        return Ok(
            json!({"model_group": null, "reason": "response id names no deployment", "hidden_by": null}),
        );
    };
    let deployments = string_map(input.get("deployments"))?;
    let Some(model_group) = deployments.get(&model_id) else {
        return Ok(
            json!({"model_group": null, "reason": "deployment no longer in the router", "hidden_by": null}),
        );
    };
    let hidden_by = hidden_by(model_group, input.get("aliases"))?;
    Ok(json!({"model_group": model_group, "reason": null, "hidden_by": hidden_by}))
}

fn submit_model_hidden_by(input: &Value) -> HelperResult {
    let model_group = input
        .get("model_group")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_input("model_group must be a string"))?;
    Ok(json!({"hidden_by": hidden_by(model_group, input.get("aliases"))?}))
}

fn hidden_by(model_group: &str, value: Option<&Value>) -> Result<Option<String>, Box<HelperError>> {
    if model_group.contains('*') {
        return Ok(Some("a wildcard deployment".to_owned()));
    }
    let aliases = alias_map(value)?;
    let matches: Vec<&str> = aliases
        .iter()
        .filter(|(_, target)| target.as_str() == model_group)
        .map(|(alias, _)| alias.as_str())
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "the target of model_group_alias {}",
        matches.join(", ")
    )))
}

fn retrieval_context(input: &Value) -> HelperResult {
    let model_group = input
        .get("model_group")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_input("model_group must be a string"))?;
    let aliases = input.get("auth").and_then(Value::as_object);
    let team_alias = aliases
        .and_then(|value| value.get("team_alias"))
        .and_then(Value::as_str);
    let key_alias = aliases
        .and_then(|value| value.get("key_alias"))
        .and_then(Value::as_str);
    let tags = input
        .get("tags")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(
        json!({"team_alias": team_alias, "key_alias": key_alias, "model": model_group,
        "tags": if tags.is_empty() { None } else { Some(tags) }}),
    )
}

fn pipeline_filtering(input: &Value) -> HelperResult {
    let pipelines = input
        .get("pipelines")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_input("pipelines must be a list"))?;
    let attached = input
        .get("already_attached")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut selected = Vec::new();
    let mut managed = BTreeMap::new();
    for item in pipelines {
        let name = item
            .get("policy_name")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_input("pipeline policy_name must be a string"))?;
        if item.get("mode").and_then(Value::as_str) != Some("post_call")
            || attached.iter().any(|value| value.as_str() == Some(name))
        {
            continue;
        }
        if let Some(guardrails) = item.get("guardrails").and_then(Value::as_array) {
            for guardrail in guardrails.iter().filter_map(Value::as_str) {
                managed.insert(guardrail.to_owned(), Value::Null);
            }
        }
        selected.push(item.clone());
    }
    Ok(json!({"pipelines": selected, "managed_guardrails": managed.keys().collect::<Vec<_>>() }))
}

fn string_map(value: Option<&Value>) -> Result<BTreeMap<String, String>, Box<HelperError>> {
    let Some(entries) = value.and_then(Value::as_object) else {
        return Err(invalid_input("deployments must be an object"));
    };
    entries
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_owned()))
                .ok_or_else(|| invalid_input("deployment model group must be a string"))
        })
        .collect()
}

fn alias_map(value: Option<&Value>) -> Result<Vec<(String, String)>, Box<HelperError>> {
    let Some(entries) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    entries
        .iter()
        .map(|pair| {
            let Some(pair) = pair.as_array() else {
                return Err(invalid_input("alias must be a pair"));
            };
            let (Some(alias), Some(target)) = (
                pair.first().and_then(Value::as_str),
                pair.get(1).and_then(Value::as_str),
            ) else {
                return Err(invalid_input("alias pair must contain strings"));
            };
            Ok((alias.to_owned(), target.to_owned()))
        })
        .collect()
}

fn invalid_input(message: impl Into<String>) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.into(),
    })
}
