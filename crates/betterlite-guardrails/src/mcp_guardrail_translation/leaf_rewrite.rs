use std::collections::BTreeMap;

use super::json_value::McpValue;

use super::nesting_depth::{Leaf, LeafPath, PathPart};

pub fn leaf_replacements<Masked>(leaves: &[Leaf], masked: Masked) -> BTreeMap<LeafPath, String>
where
    Masked: Iterator<Item = String>,
{
    let mut replacements = BTreeMap::new();
    for (leaf, replacement) in leaves.iter().zip(masked) {
        if replacement != leaf.text {
            replacements.insert(leaf.path.clone(), replacement);
        }
    }
    replacements
}

pub fn rewrite_leaves(value: &mut McpValue, replacements: &BTreeMap<LeafPath, String>) {
    for (path, replacement) in replacements {
        if let Some(McpValue::Text(text)) = value_at(value, path) {
            replacement.clone_into(text);
        }
    }
}

fn value_at<'value>(value: &'value mut McpValue, path: &LeafPath) -> Option<&'value mut McpValue> {
    let mut current = value;
    for part in &path.0 {
        current = match (current, part) {
            (McpValue::Object(entries), PathPart::Key(key)) => entries.get_mut(key)?,
            (McpValue::Array(items), PathPart::Index(index)) => items.get_mut(*index)?,
            _ => return None,
        };
    }
    Some(current)
}
