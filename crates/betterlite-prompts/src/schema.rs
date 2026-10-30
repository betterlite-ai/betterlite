use std::collections::{BTreeMap, BTreeSet};

use crate::arguments::json::{Json, Object};

mod regex_walk;
use regex_walk::drop_regexes;

const COMBINATORS: [&str; 3] = ["allOf", "anyOf", "oneOf"];
const REJECTED_TOP_LEVEL: [&str; 3] = ["enum", "const", "not"];
const MAX_FLATTEN_DEPTH: usize = 32;
const MAX_FLATTEN_WORK: usize = 100_000;
const MAX_FLATTEN_OUTPUT_BYTES: usize = 1_000_000;

#[must_use]
pub fn flatten_top_level(schema: &Object) -> Object {
    let seen_refs = BTreeSet::new();
    let mut expanded_refs = BTreeMap::new();
    let mut budget = FlattenBudget::default();
    flatten_against_root(
        schema,
        schema,
        &seen_refs,
        0,
        &mut expanded_refs,
        &mut budget,
    )
}

#[must_use]
pub fn drop_non_python_regex_patterns(schema: &Object) -> Object {
    let mut result = schema.clone();
    drop_regexes(&mut result, 0);
    result
}

#[must_use]
pub fn flatten_and_drop(schema: &Object) -> Object {
    let filtered = drop_non_python_regex_patterns(schema);
    flatten_top_level(&filtered)
}

pub fn tool_with_sanitized_parameters(tool: Object, sanitize: fn(&Object) -> Object) -> Object {
    let Some(Json::Object(function)) = tool.get("function") else {
        return tool;
    };
    let Some(Json::Object(parameters)) = function.get("parameters") else {
        return tool;
    };
    let sanitized = sanitize(parameters);
    let mut updated_function = function.clone();
    updated_function.insert("parameters".to_owned(), Json::Object(sanitized));
    let mut updated_tool = tool;
    updated_tool.insert("function".to_owned(), Json::Object(updated_function));
    updated_tool
}

fn schema_properties(schema: &Object) -> Object {
    match schema.get("properties") {
        Some(Json::Object(properties)) => properties.clone(),
        _ => Object::new(),
    }
}

fn raw_schema_branches<'schema>(
    schema: &'schema Object,
    combinator: &str,
) -> Option<Vec<&'schema Json>> {
    match schema.get(combinator) {
        Some(Json::List(branches)) => Some(branches.iter().collect()),
        _ => None,
    }
}

fn required_names(schema: &Object) -> BTreeSet<String> {
    match schema.get("required") {
        Some(Json::List(names)) => names
            .iter()
            .filter_map(Json::as_str)
            .map(ToOwned::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn flatten_against_root(
    schema: &Object,
    root: &Object,
    seen_refs: &BTreeSet<String>,
    depth: usize,
    expanded_refs: &mut BTreeMap<String, Option<Object>>,
    budget: &mut FlattenBudget,
) -> Object {
    if budget.charge_work().is_none() {
        return schema.clone();
    }
    let mut groups = Vec::new();
    for combinator in COMBINATORS {
        let Some(raw_branches) = raw_schema_branches(schema, combinator) else {
            continue;
        };
        let Some(branches) =
            flatten_group(root, raw_branches, seen_refs, depth, expanded_refs, budget)
        else {
            return schema.clone();
        };
        groups.push((combinator, branches));
    }
    let has_rejected = REJECTED_TOP_LEVEL
        .iter()
        .any(|key| schema.contains_key(*key));
    if groups.is_empty() && !has_rejected {
        return schema.clone();
    }
    let branches: Vec<&Object> = groups
        .iter()
        .flat_map(|(_, values)| values.iter())
        .collect();
    if !is_flattenable_object(schema, &branches) {
        return schema.clone();
    }
    let properties = merged_properties(schema, &branches);
    let required = merged_required(schema, &groups);
    let mut output = kept_fields(schema);
    output.insert("type".to_owned(), Json::Text("object".to_owned()));
    output.insert("properties".to_owned(), Json::Object(properties));
    if !required.is_empty() {
        output.insert(
            "required".to_owned(),
            Json::List(required.into_iter().map(Json::Text).collect()),
        );
    }
    if budget.charge_output(&output).is_none() {
        schema.clone()
    } else {
        output
    }
}

fn is_flattenable_object(schema: &Object, branches: &[&Object]) -> bool {
    let has_object_type = schema
        .get("type")
        .is_some_and(|value| value.as_str() == Some("object"));
    let has_properties_without_type =
        schema.get("type").is_none() && schema.contains_key("properties");
    if has_object_type || has_properties_without_type {
        return true;
    }
    schema.get("type").is_none()
        && !branches.is_empty()
        && branches.iter().all(|branch| is_object_schema(branch))
}

fn kept_fields(schema: &Object) -> Object {
    let mut fields = Object::new();
    for (key, value) in schema {
        let flattened = COMBINATORS.contains(&key.as_str()) && matches!(value, Json::List(_));
        if !flattened && !REJECTED_TOP_LEVEL.contains(&key.as_str()) {
            fields.insert(key.clone(), value.clone());
        }
    }
    fields
}

fn flatten_group(
    root: &Object,
    branches: Vec<&Json>,
    seen_refs: &BTreeSet<String>,
    depth: usize,
    expanded_refs: &mut BTreeMap<String, Option<Object>>,
    budget: &mut FlattenBudget,
) -> Option<Vec<Object>> {
    let next_depth = depth.checked_add(1)?;
    branches
        .into_iter()
        .map(|json| {
            let branch = json.as_object()?;
            mergeable_object(root, branch, seen_refs, next_depth, expanded_refs, budget)
        })
        .collect()
}

fn merged_properties(schema: &Object, branches: &[&Object]) -> Object {
    let mut properties = Object::new();
    for branch in branches.iter().rev() {
        for (name, value) in schema_properties(branch) {
            properties.insert(name, value);
        }
    }
    for (name, value) in schema_properties(schema) {
        properties.insert(name, value);
    }
    properties
}

fn merged_required(schema: &Object, groups: &[(&str, Vec<Object>)]) -> BTreeSet<String> {
    let mut required = required_names(schema);
    for (combinator, group) in groups {
        let branch_required: Vec<BTreeSet<String>> = group.iter().map(required_names).collect();
        let Some(first) = branch_required.first() else {
            continue;
        };
        let names = if *combinator == "allOf" {
            union_required(first, &branch_required)
        } else {
            intersection_required(first, &branch_required)
        };
        required.extend(names);
    }
    required
}

fn union_required(first: &BTreeSet<String>, groups: &[BTreeSet<String>]) -> BTreeSet<String> {
    let mut result = first.clone();
    for names in groups {
        result.extend(names.iter().cloned());
    }
    result
}

fn intersection_required(
    first: &BTreeSet<String>,
    groups: &[BTreeSet<String>],
) -> BTreeSet<String> {
    let mut required = first.clone();
    for names in groups.iter().skip(1) {
        required = required.intersection(names).cloned().collect();
    }
    required
}

fn mergeable_object(
    root: &Object,
    branch: &Object,
    seen_refs: &BTreeSet<String>,
    depth: usize,
    expanded_refs: &mut BTreeMap<String, Option<Object>>,
    budget: &mut FlattenBudget,
) -> Option<Object> {
    if depth > MAX_FLATTEN_DEPTH {
        return None;
    }
    if let Some(Json::Text(reference)) = branch.get("$ref") {
        if let Some(expanded) = expanded_refs.get(reference) {
            return expanded.clone();
        }
        if seen_refs.contains(reference) {
            return None;
        }
        let target = resolve_local_ref(root, reference)?;
        let mut next_seen = seen_refs.clone();
        next_seen.insert(reference.clone());
        let next_depth = depth.checked_add(1)?;
        let expanded =
            mergeable_object(root, target, &next_seen, next_depth, expanded_refs, budget);
        expanded_refs.insert(reference.clone(), expanded.clone());
        return expanded;
    }
    let flattened = flatten_against_root(branch, root, seen_refs, depth, expanded_refs, budget);
    if COMBINATORS.iter().any(|key| flattened.contains_key(*key)) {
        None
    } else {
        Some(flattened)
    }
}

#[derive(Default)]
struct FlattenBudget {
    work: usize,
    output_bytes: usize,
}

impl FlattenBudget {
    fn charge_work(&mut self) -> Option<()> {
        self.work = self.work.checked_add(1)?;
        (self.work <= MAX_FLATTEN_WORK).then_some(())
    }

    fn charge_output(&mut self, object: &Object) -> Option<()> {
        self.output_bytes = self
            .output_bytes
            .checked_add(estimated_object_bytes(object))?;
        (self.output_bytes <= MAX_FLATTEN_OUTPUT_BYTES).then_some(())
    }
}

fn estimated_object_bytes(object: &Object) -> usize {
    let mut total: usize = 2;
    let mut pending: Vec<&Json> = object.values().collect();
    for key in object.keys() {
        total = total.saturating_add(key.len().saturating_add(4));
    }
    while let Some(value) = pending.pop() {
        let value_bytes = match value {
            Json::Object(fields) => {
                for key in fields.keys() {
                    total = total.saturating_add(key.len().saturating_add(4));
                }
                pending.extend(fields.values());
                2
            }
            Json::List(items) => {
                pending.extend(items);
                items.len().saturating_sub(1).saturating_add(2)
            }
            Json::Text(text) => text.len().saturating_add(2),
            Json::Bool(true) | Json::Null => 4,
            Json::Bool(false) => 5,
            Json::Number(_) => 24,
        };
        total = total.saturating_add(value_bytes);
    }
    total
}

fn resolve_local_ref<'root>(root: &'root Object, reference: &str) -> Option<&'root Object> {
    let (prefix, container) = reference
        .strip_prefix("#/$defs/")
        .map(|name| (name, "$defs"))
        .or_else(|| {
            reference
                .strip_prefix("#/definitions/")
                .map(|name| (name, "definitions"))
        })?;
    root.get(container)?.as_object()?.get(prefix)?.as_object()
}

fn is_object_schema(schema: &Object) -> bool {
    let schema_type = schema.get("type");
    schema_type.map_or_else(
        || schema.contains_key("properties"),
        |value| value.as_str() == Some("object"),
    )
}
