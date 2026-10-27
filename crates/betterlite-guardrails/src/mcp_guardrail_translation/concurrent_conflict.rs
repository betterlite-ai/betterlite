use super::nesting_depth::{Leaf, LeafPath};
use std::collections::BTreeMap;

mod tests;

// parallel guardrails scan one snapshot, so a leaf holding neither text was rewritten by another one
#[must_use]
pub fn conflicting_paths(
    scanned: &[Leaf],
    current: &[Leaf],
    replacements: &BTreeMap<LeafPath, String>,
) -> Vec<LeafPath> {
    let reshaped = !scanned
        .iter()
        .map(|leaf| &leaf.path)
        .eq(current.iter().map(|leaf| &leaf.path));
    if reshaped {
        return replacements.keys().cloned().collect();
    }
    let mut conflicts = Vec::new();
    for (scanned_leaf, current_leaf) in scanned.iter().zip(current) {
        if let Some(replacement) = replacements.get(&scanned_leaf.path)
            && current_leaf.text != scanned_leaf.text
            && current_leaf.text != *replacement
        {
            conflicts.push(scanned_leaf.path.clone());
        }
    }
    conflicts
}
