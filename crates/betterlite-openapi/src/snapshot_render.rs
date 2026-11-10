use std::collections::BTreeMap;

use crate::schema_transforms::object_in;
use crate::schema_transforms::tree::{JsonObject, JsonTree};

mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDrift {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub unreadable: Option<String>,
}

impl SnapshotDrift {
    #[must_use]
    pub fn message(&self) -> String {
        if let Some(reason) = &self.unreadable {
            return format!("the snapshot cannot be compared: {reason}");
        }
        let mut message = "the committed snapshot differs from the rendered document".to_owned();
        if !self.added.is_empty() {
            message.push_str("\nadded paths: ");
            message.push_str(&self.added.join(", "));
        }
        if !self.removed.is_empty() {
            message.push_str("\nremoved paths: ");
            message.push_str(&self.removed.join(", "));
        }
        if !self.changed.is_empty() {
            message.push_str("\nchanged paths: ");
            message.push_str(&self.changed.join(", "));
        }
        message
    }
}

#[must_use]
pub fn render_snapshot(document: &JsonObject) -> String {
    let mut rendered = String::new();
    write_object(&mut rendered, document, 0);
    rendered.push('\n');
    rendered
}

pub fn check_snapshot(committed: &str, rendered: &str) -> Result<(), SnapshotDrift> {
    if committed == rendered {
        return Ok(());
    }
    let committed_document = parse_snapshot(committed, "committed")?;
    let rendered_document = parse_snapshot(rendered, "rendered")?;
    Err(path_drift(&committed_document, &rendered_document))
}

fn parse_snapshot(text: &str, origin: &str) -> Result<JsonObject, SnapshotDrift> {
    serde_json::from_str::<JsonObject>(text).map_err(|source| SnapshotDrift {
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
        unreadable: Some(format!("the {origin} file is not valid JSON: {source}")),
    })
}

fn path_drift(committed: &JsonObject, rendered: &JsonObject) -> SnapshotDrift {
    let committed_paths = path_values(committed);
    let rendered_paths = path_values(rendered);
    let mut added = Vec::new();
    let mut changed = Vec::new();
    for (path, rendered_item) in &rendered_paths {
        match committed_paths.get(path) {
            None => added.push((*path).to_owned()),
            Some(committed_item) => {
                if committed_item != rendered_item {
                    changed.push((*path).to_owned());
                }
            }
        }
    }
    let removed = committed_paths
        .keys()
        .filter(|path| !rendered_paths.contains_key(*path))
        .map(|path| (*path).to_owned())
        .collect();
    SnapshotDrift {
        added,
        removed,
        changed,
        unreadable: None,
    }
}

fn path_values(document: &JsonObject) -> BTreeMap<&str, &JsonTree> {
    document
        .get("paths")
        .and_then(object_in)
        .map(|paths| {
            paths
                .iter()
                .map(|(path, item)| (path.as_str(), item))
                .collect()
        })
        .unwrap_or_default()
}

fn write_tree(output: &mut String, tree: &JsonTree, depth: usize) {
    match tree {
        JsonTree::Null => output.push_str("null"),
        JsonTree::Flag(value) => output.push_str(if *value { "true" } else { "false" }),
        JsonTree::Number(number) => output.push_str(&number.to_string()),
        JsonTree::Text(text) => write_string(output, text),
        JsonTree::Array(items) => write_array(output, items, depth),
        JsonTree::Object(object) => write_object(output, object, depth),
    }
}

fn write_array(output: &mut String, items: &[JsonTree], depth: usize) {
    if items.is_empty() {
        output.push_str("[]");
        return;
    }
    output.push('[');
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push('\n');
        output.push_str(&"  ".repeat(depth.saturating_add(1)));
        write_tree(output, item, depth.saturating_add(1));
    }
    output.push('\n');
    output.push_str(&"  ".repeat(depth));
    output.push(']');
}

fn write_object(output: &mut String, object: &JsonObject, depth: usize) {
    if object.is_empty() {
        output.push_str("{}");
        return;
    }
    let mut keys = object.keys().collect::<Vec<&String>>();
    keys.sort_unstable();
    output.push('{');
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push('\n');
        output.push_str(&"  ".repeat(depth.saturating_add(1)));
        write_string(output, key.as_str());
        output.push_str(": ");
        if let Some(value) = object.get(*key) {
            write_tree(output, value, depth.saturating_add(1));
        }
    }
    output.push('\n');
    output.push_str(&"  ".repeat(depth));
    output.push('}');
}

fn write_string(output: &mut String, text: &str) {
    output.push('"');
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0C}' => output.push_str("\\f"),
            control if control < ' ' => {
                let hex = format!("{:02x}", u32::from(control));
                output.push_str("\\u00");
                output.push_str(&hex);
            }
            other => output.push(other),
        }
    }
    output.push('"');
}
