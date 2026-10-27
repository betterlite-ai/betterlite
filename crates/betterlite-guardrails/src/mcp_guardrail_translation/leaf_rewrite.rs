use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

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

pub fn rewrite_leaves(value: &mut GuardrailValue, replacements: &BTreeMap<LeafPath, String>) {
    for (path, replacement) in replacements {
        if let Some(GuardrailValue::Text(text)) = value_at(value, path) {
            replacement.clone_into(text);
        }
    }
}

fn value_at<'value>(
    value: &'value mut GuardrailValue,
    path: &LeafPath,
) -> Option<&'value mut GuardrailValue> {
    let mut current = value;
    for part in &path.0 {
        current = match (current, part) {
            (GuardrailValue::Object(entries), PathPart::Key(key)) => entries.get_mut(key)?,
            (GuardrailValue::Array(items), PathPart::Index(index)) => items.get_mut(*index)?,
            _ => return None,
        };
    }
    Some(current)
}
