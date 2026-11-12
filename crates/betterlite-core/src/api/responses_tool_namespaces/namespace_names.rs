use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;

use super::error::NamespaceToolsError;
use super::values::{Fields, member_list, text_field, tool_objects, tool_type};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespacedName {
    pub namespace: String,
    pub name: String,
}

// (namespace, function name) of every function inside a namespace tool; custom and nested namespace members never count
fn namespace_function_entries(
    tools: &[&Fields],
) -> Result<Vec<NamespacedName>, NamespaceToolsError> {
    let mut entries = Vec::new();
    for tool in tools.iter().filter(|tool| tool_type(tool) == "namespace") {
        let namespace = text_field(tool, "name")?;
        let Some(members) = member_list(tool) else {
            continue;
        };
        for member in members {
            if let Json::Object(fields) = member
                && tool_type(fields) == "function"
            {
                entries.push(NamespacedName {
                    namespace: namespace.to_owned(),
                    name: text_field(fields, "name")?.to_owned(),
                });
            }
        }
    }
    Ok(entries)
}

fn top_level_function_names(tools: &[&Fields]) -> Result<BTreeSet<String>, NamespaceToolsError> {
    let mut names = BTreeSet::new();
    for tool in tools.iter().filter(|tool| tool_type(tool) == "function") {
        names.insert(text_field(tool, "name")?.to_owned());
    }
    Ok(names)
}

pub fn check_collisions(tools: &[Json]) -> Result<(), NamespaceToolsError> {
    let objects = tool_objects(tools)?;
    let top_level = top_level_function_names(&objects)?;
    let conflicts: BTreeSet<String> = namespace_function_entries(&objects)?
        .into_iter()
        .map(|entry| format!("{}__{}", entry.namespace, entry.name))
        .filter(|flattened| top_level.contains(flattened))
        .collect();
    if conflicts.is_empty() {
        return Ok(());
    }
    Err(NamespaceToolsError::Conflict {
        names: conflicts.into_iter().collect::<Vec<_>>().join(", "),
    })
}

// the qualified name always maps back; a bare name maps back only when exactly one namespace has it and no top-level function does
pub fn name_map(tools: &[Json]) -> Result<BTreeMap<String, NamespacedName>, NamespaceToolsError> {
    let objects = tool_objects(tools)?;
    let entries = namespace_function_entries(&objects)?;
    let top_level = top_level_function_names(&objects)?;
    let mut names = BTreeMap::new();
    for entry in &entries {
        names.insert(
            format!("{}__{}", entry.namespace, entry.name),
            entry.clone(),
        );
    }
    for entry in &entries {
        let occurrences = entries
            .iter()
            .filter(|candidate| candidate.name == entry.name)
            .count();
        if !top_level.contains(&entry.name) && occurrences == 1 {
            names.insert(entry.name.clone(), entry.clone());
        }
    }
    Ok(names)
}

#[must_use]
pub fn restore_name(
    name: &str,
    names: &BTreeMap<String, NamespacedName>,
) -> (String, Option<String>) {
    names.get(name).map_or_else(
        || (name.to_owned(), None),
        |mapped| (mapped.name.clone(), Some(mapped.namespace.clone())),
    )
}
