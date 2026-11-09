use std::collections::BTreeSet;

use crate::tag_matchers::candidates::DeploymentTags;
use crate::tag_matchers::selection::TagQuery;
use crate::tag_matchers::tokens::RequestTags;

pub(super) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn deployment(tags: Option<&[&str]>, regex: Option<&[&str]>) -> DeploymentTags {
    DeploymentTags {
        tags: tags.map(strings),
        tag_regex: regex.map(strings),
    }
}

pub(super) fn query(
    caller: &[&str],
    inherited: Option<&[&str]>,
    headers: &[&str],
    match_any: bool,
    prefix: &str,
) -> TagQuery {
    TagQuery {
        request: RequestTags {
            caller: Some(strings(caller)),
            inherited: inherited.map(strings),
        },
        headers: strings(headers),
        match_any,
        routing_prefix: prefix.to_owned(),
    }
}
