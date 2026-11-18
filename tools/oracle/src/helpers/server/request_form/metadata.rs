use betterlite_core::api::request_form::metadata::{
    MetadataTree, MetadataValue, MetadataWarning, MetadataWarningKind, extract_nested_form_metadata,
};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Map, Value, json};

use super::Input;
use super::fields::direct_fields;
use super::projection::form_value;

const DEFAULT_PREFIX: &str = "litellm_metadata[";

pub(super) fn metadata(input: &Input) -> HelperResult {
    let fields = direct_fields(input)?;
    let (tree, warnings) = match input.prefix.as_deref() {
        Some(prefix) => extract_nested_form_metadata(&fields, prefix),
        None => extract_nested_form_metadata(&fields, DEFAULT_PREFIX),
    };
    let tree_value = metadata_value(&tree)?;
    Ok(json!({
        "metadata": tree_value,
        "warnings": warnings_value(&warnings),
    }))
}

fn metadata_value(tree: &MetadataTree) -> HelperResult {
    let mut entries = Map::new();
    for (key, value) in tree {
        let projected = match value {
            MetadataValue::Leaf(leaf) => form_value(leaf)?,
            MetadataValue::Object(child) => metadata_value(child)?,
        };
        entries.insert(key.clone(), projected);
    }
    Ok(Value::Object(entries))
}

fn warnings_value(warnings: &[MetadataWarning]) -> Value {
    Value::Array(
        warnings
            .iter()
            .map(|warning| {
                json!({
                    "kind": warning_kind(warning.kind),
                    "subject": warning.subject,
                })
            })
            .collect(),
    )
}

const fn warning_kind(kind: MetadataWarningKind) -> &'static str {
    match kind {
        MetadataWarningKind::UploadSkipped => "upload_skipped",
        MetadataWarningKind::EmptyPath => "empty_path",
        MetadataWarningKind::IntermediateNotObject => "intermediate_not_object",
        MetadataWarningKind::ParentNotObject => "parent_not_object",
        MetadataWarningKind::PathTooDeep => "path_too_deep",
    }
}
