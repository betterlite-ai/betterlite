use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

// Caller tags and trusted inherited constraints stay apart so policy can use the snapshot untouched
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTags {
    #[serde(default)]
    pub caller: Option<Vec<String>>,
    #[serde(default)]
    pub inherited: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagParts {
    pub required: Vec<String>,
    pub positive: Vec<String>,
    pub excluded: Vec<String>,
}

#[must_use]
pub fn bare_tag_value(tag: &str) -> Option<&str> {
    tag.strip_prefix('&')
        .or_else(|| tag.strip_prefix('!'))
        .map_or(Some(tag), |value| (!value.is_empty()).then_some(value))
}

#[must_use]
pub fn split_tags(tags: &[String]) -> TagParts {
    let mut parts = TagParts::default();
    for tag in tags {
        if let Some(required) = tag.strip_prefix('&') {
            if !required.is_empty() {
                parts.required.push(required.to_owned());
            }
        } else if let Some(excluded) = tag.strip_prefix('!') {
            if !excluded.is_empty() {
                parts.excluded.push(excluded.to_owned());
            }
        } else {
            parts.positive.push(tag.clone());
        }
    }
    parts
}

#[must_use]
pub fn strip_routing_prefix(tags: &[String], prefix: &str) -> (Vec<String>, BTreeSet<String>) {
    if prefix.is_empty() {
        return (tags.to_vec(), BTreeSet::new());
    }
    let mut rewritten = Vec::with_capacity(tags.len());
    let mut confirmed = BTreeSet::new();
    for tag in tags {
        // removeprefix keeps a tag that does not carry the configured marker exactly as sent
        let Some(value) = tag.strip_prefix(prefix) else {
            rewritten.push(tag.clone());
            continue;
        };
        rewritten.push(value.to_owned());
        if let Some(bare) = bare_tag_value(value) {
            confirmed.insert(bare.to_owned());
        }
    }
    (rewritten, confirmed)
}
