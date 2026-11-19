use std::collections::BTreeMap;

use betterlite_prometheus::label_sanitization::sanitize_label_name;

use super::tag_wildcard_matching::{TagPatternError, tag_matches_wildcard_pattern};

pub fn custom_labels_from_tags(
    tags: &[String],
    configured_tags: &[String],
) -> Result<BTreeMap<String, String>, TagPatternError> {
    let mut labels = BTreeMap::new();
    for configured in configured_tags {
        let label_name = sanitize_label_name(&format!("tag_{configured}"));
        if tags.iter().any(|tag| tag == configured) {
            labels.insert(label_name, "true".to_owned());
            continue;
        }
        let matched = configured.contains('*') && tag_matches_wildcard_pattern(tags, configured)?;
        labels.insert(
            label_name,
            if matched { "true" } else { "false" }.to_owned(),
        );
    }
    Ok(labels)
}

#[path = "tag_labels/tests.rs"]
mod tests;
