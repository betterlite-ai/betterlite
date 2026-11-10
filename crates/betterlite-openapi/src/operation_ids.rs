use std::collections::{BTreeMap, BTreeSet};

use crate::schema_transforms::tree::{JsonObject, JsonTree};

mod tests;

const OPENAPI_HTTP_METHODS: [&str; 8] = [
    "delete", "get", "head", "options", "patch", "post", "put", "trace",
];

struct OperationEntry {
    path: String,
    method: String,
    operation_id: String,
}

#[must_use]
pub fn stable_operation_id(name: &str, path_format: &str, methods: &[&str]) -> String {
    let mut operation_id = String::with_capacity(name.len().saturating_add(path_format.len()));
    for character in name.chars().chain(path_format.chars()) {
        if character.is_alphanumeric() || character == '_' {
            operation_id.push(character);
        } else {
            operation_id.push('_');
        }
    }
    let mut sorted_methods = methods.to_vec();
    sorted_methods.sort_unstable();
    if let [method] = sorted_methods.as_slice() {
        operation_id.push('_');
        operation_id.push_str(&method.to_lowercase());
    }
    operation_id
}

pub fn ensure_unique_operation_ids(
    document: &mut JsonObject,
    reserved: Option<&mut BTreeSet<String>>,
) {
    let entries = collect_operations(document);
    let counts = operation_id_counts(&entries);
    let mut used = reserved
        .as_ref()
        .map(|operation_ids| (*operation_ids).clone())
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    for entry in &entries {
        rewrite_operation(document, entry, &counts, &mut used, &mut seen);
    }
    if let Some(reserved_ids) = reserved {
        reserved_ids.extend(used);
    }
}

fn collect_operations(document: &JsonObject) -> Vec<OperationEntry> {
    let Some(JsonTree::Object(paths)) = document.get("paths") else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for (path, path_item) in paths {
        collect_path_operations(path, path_item, &mut entries);
    }
    entries
}

fn collect_path_operations(path: &str, path_item: &JsonTree, entries: &mut Vec<OperationEntry>) {
    let JsonTree::Object(operations) = path_item else {
        return;
    };
    for (method, operation) in operations {
        let Some(operation_id) = operation_id_of(method, operation) else {
            continue;
        };
        entries.push(OperationEntry {
            path: path.to_owned(),
            method: method.clone(),
            operation_id: operation_id.clone(),
        });
    }
}

fn operation_id_of<'tree>(method: &str, operation: &'tree JsonTree) -> Option<&'tree String> {
    if !OPENAPI_HTTP_METHODS.contains(&method) {
        return None;
    }
    let JsonTree::Object(operation_object) = operation else {
        return None;
    };
    let JsonTree::Text(operation_id) = operation_object.get("operationId")? else {
        return None;
    };
    Some(operation_id)
}

fn operation_id_counts(entries: &[OperationEntry]) -> BTreeMap<String, u8> {
    let mut counts: BTreeMap<String, u8> = BTreeMap::new();
    for entry in entries {
        counts
            .entry(entry.operation_id.clone())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
    }
    counts
}

fn rewrite_operation(
    document: &mut JsonObject,
    entry: &OperationEntry,
    counts: &BTreeMap<String, u8>,
    used: &mut BTreeSet<String>,
    seen: &mut BTreeSet<String>,
) {
    let repeated = counts
        .get(&entry.operation_id)
        .is_some_and(|count| *count > 1);
    if !repeated && !used.contains(&entry.operation_id) && !seen.contains(&entry.operation_id) {
        seen.insert(entry.operation_id.clone());
        used.insert(entry.operation_id.clone());
        return;
    }
    let base = strip_method_suffix(&entry.operation_id);
    let mut candidate = format!("{base}_{}", entry.method);
    let mut suffix: u32 = 2;
    while used.contains(&candidate) || seen.contains(&candidate) {
        candidate = format!("{base}_{}_{suffix}", entry.method);
        suffix = suffix.saturating_add(1);
    }
    set_operation_id(document, entry, &candidate);
    seen.insert(candidate.clone());
    used.insert(candidate);
}

fn set_operation_id(document: &mut JsonObject, entry: &OperationEntry, operation_id: &str) {
    let Some(JsonTree::Object(paths)) = document.get_mut("paths") else {
        return;
    };
    let Some(JsonTree::Object(path_item)) = paths.get_mut(&entry.path) else {
        return;
    };
    let Some(JsonTree::Object(operation)) = path_item.get_mut(&entry.method) else {
        return;
    };
    operation.insert(
        "operationId".to_owned(),
        JsonTree::Text(operation_id.to_owned()),
    );
}

fn strip_method_suffix(operation_id: &str) -> String {
    let Some((base, suffix)) = operation_id.rsplit_once('_') else {
        return operation_id.to_owned();
    };
    if OPENAPI_HTTP_METHODS.contains(&suffix) {
        return base.to_owned();
    }
    operation_id.to_owned()
}
