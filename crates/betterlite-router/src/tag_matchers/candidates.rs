use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::patterns::{Patterns, TagWarning};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentTags {
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub tag_regex: Option<Vec<String>>,
}

#[derive(Debug)]
pub struct Deployment {
    pub tags: Option<Vec<String>>,
    pub patterns: Patterns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchVia {
    Tags,
    TagRegex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentMatch {
    pub index: usize,
    pub via: MatchVia,
    pub value: String,
}

#[derive(Debug, Default)]
pub struct MatchResults {
    pub matches: Vec<DeploymentMatch>,
    pub warnings: Vec<TagWarning>,
}

#[derive(Debug, Default)]
pub struct FoundMatch {
    pub matched: Option<(MatchVia, String)>,
    pub warnings: Vec<TagWarning>,
}

#[must_use]
pub fn is_valid_deployment_tag(
    deployment_tags: &[String],
    request_tags: &[String],
    match_any: bool,
) -> bool {
    if request_tags.is_empty() {
        return false;
    }
    let present: BTreeSet<&String> = deployment_tags.iter().collect();
    if match_any {
        request_tags.iter().any(|tag| present.contains(tag))
    } else {
        request_tags.iter().all(|tag| present.contains(tag))
    }
}

impl Deployment {
    #[must_use]
    pub fn matching(&self, positive: &[String], headers: &[String], match_any: bool) -> FoundMatch {
        let empty: &[String] = &[];
        let tags = self.tags.as_deref().unwrap_or(empty);
        let mut result = FoundMatch::default();
        if !tags.is_empty()
            && !positive.is_empty()
            && is_valid_deployment_tag(tags, positive, match_any)
        {
            let requested: BTreeSet<&String> = positive.iter().collect();
            let matched_value = tags
                .iter()
                .find(|tag| requested.contains(tag))
                .or_else(|| tags.first());
            if let Some(value) = matched_value {
                result.matched = Some((MatchVia::Tags, value.clone()));
            }
            return result;
        }
        // a failed strict all-tags check blocks the header path, so a regex cannot override it
        let strict_failed = !match_any && !tags.is_empty();
        if !headers.is_empty() && !strict_failed {
            let search = self.patterns.find(headers);
            result.warnings = search.warnings;
            if let Some(pattern) = search.matched {
                result.matched = Some((MatchVia::TagRegex, pattern));
            }
        }
        result
    }
}

#[must_use]
pub fn exclude(
    deployments: &[Deployment],
    pool: &[usize],
    excluded: &BTreeSet<String>,
) -> Vec<usize> {
    if excluded.is_empty() {
        return pool.to_vec();
    }
    let mut kept = Vec::new();
    for index in pool {
        let Some(deployment) = deployments.get(*index) else {
            continue;
        };
        let carries_excluded = deployment
            .tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|tag| excluded.contains(tag)));
        if !carries_excluded {
            kept.push(*index);
        }
    }
    kept
}

#[must_use]
pub fn require_all(
    deployments: &[Deployment],
    pool: &[usize],
    required: &BTreeSet<String>,
) -> Vec<usize> {
    if required.is_empty() {
        return pool.to_vec();
    }
    let mut kept = Vec::new();
    for index in pool {
        let Some(deployment) = deployments.get(*index) else {
            continue;
        };
        let has_all = deployment.tags.as_ref().is_some_and(|tags| {
            let present: BTreeSet<&String> = tags.iter().collect();
            required.iter().all(|tag| present.contains(tag))
        });
        if has_all {
            kept.push(*index);
        }
    }
    kept
}

#[must_use]
pub fn default_tagged(deployments: &[Deployment], pool: &[usize]) -> Vec<usize> {
    let mut defaults = Vec::new();
    for index in pool {
        let Some(deployment) = deployments.get(*index) else {
            continue;
        };
        let tagged = deployment
            .tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|tag| tag == "default"));
        if tagged {
            defaults.push(*index);
        }
    }
    if defaults.is_empty() {
        return pool.to_vec();
    }
    defaults
}

#[must_use]
pub fn known_tag_values(deployments: &[Deployment]) -> BTreeSet<String> {
    let mut known = BTreeSet::new();
    for deployment in deployments {
        if let Some(tags) = &deployment.tags {
            known.extend(tags.iter().cloned());
        }
    }
    known
}
