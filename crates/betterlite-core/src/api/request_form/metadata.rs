use indexmap::IndexMap;

use super::values::{FormFields, FormValue};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataValue {
    Leaf(FormValue),
    Object(MetadataTree),
}

pub type MetadataTree = IndexMap<String, MetadataValue>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataWarningKind {
    UploadSkipped,
    EmptyPath,
    IntermediateNotObject,
    ParentNotObject,
    PathTooDeep,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataWarning {
    pub kind: MetadataWarningKind,
    pub subject: String,
}

// extract_nested_form_metadata: litellm_metadata[a][b] nests, a scalar parent drops a deeper path with a warning, and a later scalar leaf overwrites an earlier subtree silently.
#[must_use]
pub fn extract_nested_form_metadata(
    fields: &FormFields,
    prefix: &str,
) -> (MetadataTree, Vec<MetadataWarning>) {
    let mut metadata = MetadataTree::new();
    let mut warnings = Vec::new();
    for (key, value) in fields {
        if !key.starts_with(prefix) {
            continue;
        }
        if matches!(value, FormValue::Upload(_)) {
            warnings.push(warning(MetadataWarningKind::UploadSkipped, key));
            continue;
        }
        let path = key.replace(prefix, "");
        let path = path.trim_end_matches(']');
        let first = path.split("][").next().unwrap_or_default();
        if first.is_empty() {
            warnings.push(warning(MetadataWarningKind::EmptyPath, key));
            continue;
        }
        // Bound the tree itself: recursive cloning and destruction must also fit the worker stack.
        let parts: Vec<&str> = path.split("][").take(129).collect();
        if parts.len() > 128 {
            warnings.push(warning(MetadataWarningKind::PathTooDeep, key));
            continue;
        }
        place(&mut metadata, &parts, key, value, &mut warnings);
    }
    (metadata, warnings)
}

fn warning(kind: MetadataWarningKind, subject: &str) -> MetadataWarning {
    MetadataWarning {
        kind,
        subject: subject.to_owned(),
    }
}

fn place(
    tree: &mut MetadataTree,
    parts: &[&str],
    key: &str,
    value: &FormValue,
    warnings: &mut Vec<MetadataWarning>,
) {
    let Some((head, tail)) = parts.split_first() else {
        return;
    };
    if tail.is_empty() {
        tree.insert((*head).to_owned(), MetadataValue::Leaf(value.clone()));
        return;
    }
    let child = tree
        .entry((*head).to_owned())
        .or_insert_with(|| MetadataValue::Object(MetadataTree::new()));
    match child {
        MetadataValue::Object(map) => place(map, tail, key, value, warnings),
        MetadataValue::Leaf(_) => {
            if tail.len() == 1 {
                warnings.push(warning(MetadataWarningKind::ParentNotObject, key));
            } else {
                let subject = tail.first().copied().unwrap_or_default();
                warnings.push(warning(MetadataWarningKind::IntermediateNotObject, subject));
            }
        }
    }
}
