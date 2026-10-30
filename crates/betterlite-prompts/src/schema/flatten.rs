use std::collections::{BTreeMap, BTreeSet};

use super::budget::FlattenBudget;
use super::flatten_and_drop_schema;
use crate::arguments::json::{Json, Object};

const COMBINATORS: [&str; 3] = ["allOf", "anyOf", "oneOf"];
const REJECTED_TOP_LEVEL: [&str; 3] = ["enum", "const", "not"];
const MAX_FLATTEN_DEPTH: usize = 32;

#[must_use]
pub fn flatten_top_level(schema: &Object) -> Object {
    let mut context = FlattenContext {
        root: schema,
        expanded_refs: BTreeMap::new(),
        budget: FlattenBudget::default(),
    };
    flatten_against_root(schema, &BTreeSet::new(), 0, &mut context)
}

#[must_use]
pub fn flatten_and_drop(schema: &Object) -> Object {
    let filtered = flatten_and_drop_schema(schema);
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

struct FlattenContext<'root> {
    root: &'root Object,
    expanded_refs: BTreeMap<String, Option<Object>>,
    budget: FlattenBudget,
}

fn flatten_against_root(
    schema: &Object,
    seen_refs: &BTreeSet<String>,
    depth: usize,
    context: &mut FlattenContext<'_>,
) -> Object {
    if context.budget.charge_work().is_none() {
        return schema.clone();
    }
    let Some(groups) = collect_groups(schema, seen_refs, depth, context) else {
        return schema.clone();
    };
    let has_rejected = REJECTED_TOP_LEVEL
        .iter()
        .any(|key| schema.contains_key(*key));
    if groups.is_empty() && !has_rejected {
        return schema.clone();
    }
    flatten_object(schema, &groups, context)
}

fn collect_groups(
    schema: &Object,
    seen_refs: &BTreeSet<String>,
    depth: usize,
    context: &mut FlattenContext<'_>,
) -> Option<Vec<(&'static str, Vec<Object>)>> {
    let mut groups = Vec::new();
    for combinator in COMBINATORS {
        let Some(raw_branches) = raw_schema_branches(schema, combinator) else {
            continue;
        };
        let branches = flatten_group(raw_branches, seen_refs, depth, context)?;
        groups.push((combinator, branches));
    }
    Some(groups)
}

fn flatten_object(
    schema: &Object,
    groups: &[(&str, Vec<Object>)],
    context: &mut FlattenContext<'_>,
) -> Object {
    let branches: Vec<&Object> = groups
        .iter()
        .flat_map(|(_, values)| values.iter())
        .collect();
    if !is_flattenable_object(schema, &branches) {
        return schema.clone();
    }
    let properties = merged_properties(schema, &branches);
    let required = merged_required(schema, groups);
    let mut output = kept_fields(schema);
    output.insert("type".to_owned(), Json::Text("object".to_owned()));
    output.insert("properties".to_owned(), Json::Object(properties));
    if !required.is_empty() {
        output.insert(
            "required".to_owned(),
            Json::List(required.into_iter().map(Json::Text).collect()),
        );
    }
    if context.budget.charge_output(&output).is_none() {
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
    branches: Vec<&Json>,
    seen_refs: &BTreeSet<String>,
    depth: usize,
    context: &mut FlattenContext<'_>,
) -> Option<Vec<Object>> {
    let next_depth = depth.checked_add(1)?;
    branches
        .into_iter()
        .map(|json| {
            let branch = json.as_object()?;
            mergeable_object(branch, seen_refs, next_depth, context)
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
    branch: &Object,
    seen_refs: &BTreeSet<String>,
    depth: usize,
    context: &mut FlattenContext<'_>,
) -> Option<Object> {
    if depth > MAX_FLATTEN_DEPTH {
        return None;
    }
    if let Some(Json::Text(reference)) = branch.get("$ref") {
        if let Some(expanded) = context.expanded_refs.get(reference) {
            return expanded.clone();
        }
        if seen_refs.contains(reference) {
            return None;
        }
        let target = resolve_local_ref(context.root, reference)?;
        let mut next_seen = seen_refs.clone();
        next_seen.insert(reference.clone());
        let next_depth = depth.checked_add(1)?;
        let expanded = mergeable_object(target, &next_seen, next_depth, context);
        context
            .expanded_refs
            .insert(reference.clone(), expanded.clone());
        return expanded;
    }
    let flattened = flatten_against_root(branch, seen_refs, depth, context);
    if COMBINATORS.iter().any(|key| flattened.contains_key(*key)) {
        None
    } else {
        Some(flattened)
    }
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

mod tests;
