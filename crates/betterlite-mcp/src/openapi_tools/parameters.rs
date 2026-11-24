use std::collections::BTreeSet;

use indexmap::IndexMap;

use crate::openapi_tools::document::{Components, Operation, Parameter, PathItem};

const COMPONENT_PREFIX: &str = "#/components/parameters/";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedParameters {
    pub path: Vec<String>,
    pub query: Vec<String>,
    pub body: Vec<String>,
}

// LiteLLM's resolve_operation_params: component refs resolved, nameless entries dropped, operation level winning
#[must_use]
pub fn resolve_operation_params(
    operation: &Operation,
    path_item: &PathItem,
    components: &Components,
) -> Operation {
    let component_params = &components.parameters;
    let path_level = resolve_param_list(&path_item.parameters, component_params);
    let op_level = resolve_param_list(&operation.parameters, component_params);
    let op_keys: BTreeSet<(String, Option<String>)> = op_level
        .iter()
        .map(|param| {
            (
                param.name.clone().unwrap_or_default(),
                param.location.clone(),
            )
        })
        .collect();
    let mut merged: Vec<Parameter> = path_level
        .into_iter()
        .filter(|param| {
            let key = (
                param.name.clone().unwrap_or_default(),
                param.location.clone(),
            );
            !op_keys.contains(&key)
        })
        .collect();
    merged.extend(op_level);
    let mut resolved = operation.clone();
    resolved.parameters = merged;
    resolved
}

fn resolve_param_list(
    raw: &[Parameter],
    component_params: &IndexMap<String, Parameter>,
) -> Vec<Parameter> {
    let mut resolved = Vec::with_capacity(raw.len());
    for param in raw {
        let Some(found) = resolve_ref(param, component_params) else {
            continue;
        };
        if found.name.as_deref().is_none_or(str::is_empty) {
            continue;
        }
        resolved.push(found);
    }
    resolved
}

fn resolve_ref(
    param: &Parameter,
    component_params: &IndexMap<String, Parameter>,
) -> Option<Parameter> {
    let reference = param.reference.as_deref().unwrap_or_default();
    if !reference.starts_with(COMPONENT_PREFIX) {
        return Some(param.clone());
    }
    let last = reference.rsplit('/').next().unwrap_or_default();
    component_params.get(last).cloned()
}

// LiteLLM's extract_parameters: only path, query and body parameters, plus "body" for any requestBody
#[must_use]
pub fn extract_parameters(operation: &Operation) -> ExtractedParameters {
    let mut extracted = ExtractedParameters::default();
    for param in &operation.parameters {
        let Some(name) = param.name.clone() else {
            continue;
        };
        match param.location.as_deref() {
            Some("path") => extracted.path.push(name),
            Some("query") => extracted.query.push(name),
            Some("body") => extracted.body.push(name),
            Some(_) | None => {}
        }
    }
    if operation.request_body.is_some() {
        extracted.body.push("body".to_owned());
    }
    extracted
}
