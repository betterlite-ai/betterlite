use std::collections::BTreeSet;

use super::candidates::{
    Deployment, DeploymentMatch, DeploymentTags, MatchResults, default_tagged, exclude,
    known_tag_values, require_all,
};
use super::patterns::{Patterns, TagWarning};
use super::tokens::{RequestTags, TagParts, split_tags, strip_routing_prefix};

#[derive(Debug)]
pub struct TagMatcher {
    pub deployments: Vec<Deployment>,
    pub warnings: Vec<TagWarning>,
}

#[derive(Debug, Clone, Default)]
pub struct TagQuery {
    pub request: RequestTags,
    pub headers: Vec<String>,
    pub match_any: bool,
    pub routing_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSelection {
    pub rewritten: Vec<String>,
    pub tags: TagParts,
    pub routing_confirmed: BTreeSet<String>,
    pub inherited_required: Option<BTreeSet<String>>,
    pub inherited_excluded: Option<BTreeSet<String>>,
    pub allowed: Vec<usize>,
    pub candidates: Vec<usize>,
    pub matches: Vec<DeploymentMatch>,
    pub defaults: Vec<usize>,
    pub known_tags: BTreeSet<String>,
    pub unknown_required_hides_answer: bool,
    pub warnings: Vec<TagWarning>,
}

impl TagMatcher {
    #[must_use]
    pub fn new(deployments: Vec<DeploymentTags>) -> Self {
        let mut compiled = Vec::with_capacity(deployments.len());
        let mut warnings = Vec::new();
        for deployment in deployments {
            let regexes = deployment.tag_regex.as_deref().unwrap_or(&[]);
            let patterns = Patterns::compile(regexes);
            warnings.extend(patterns.warnings.iter().cloned());
            compiled.push(Deployment {
                tags: deployment.tags,
                patterns,
            });
        }
        Self {
            deployments: compiled,
            warnings,
        }
    }

    #[must_use]
    pub fn select(&self, query: &TagQuery) -> TagSelection {
        let caller = query.request.caller.as_deref().unwrap_or(&[]);
        let (rewritten, routing_confirmed) = strip_routing_prefix(caller, &query.routing_prefix);
        let tags = split_tags(&rewritten);
        let required: BTreeSet<String> = tags.required.iter().cloned().collect();
        let excluded: BTreeSet<String> = tags.excluded.iter().cloned().collect();
        let (inherited_required, inherited_excluded) =
            inherited_sets(query.request.inherited.as_deref(), &query.routing_prefix);
        let pool: Vec<usize> = (0..self.deployments.len()).collect();
        let allowed = exclude(&self.deployments, &pool, &excluded);
        let candidates = require_all(&self.deployments, &allowed, &required);
        let matched = self.match_pool(&candidates, &tags.positive, &query.headers, query.match_any);
        TagSelection {
            rewritten,
            routing_confirmed: routing_confirmed.clone(),
            unknown_required_hides_answer: self.unknown_required_hides_answer(
                &pool,
                &excluded,
                &required,
                &routing_confirmed,
            ),
            tags,
            inherited_required,
            inherited_excluded,
            allowed,
            candidates,
            matches: matched.matches,
            defaults: default_tagged(&self.deployments, &pool),
            known_tags: known_tag_values(&self.deployments, &pool),
            warnings: matched.warnings,
        }
    }

    #[must_use]
    pub fn match_pool(
        &self,
        pool: &[usize],
        positive: &[String],
        headers: &[String],
        match_any: bool,
    ) -> MatchResults {
        let mut results = MatchResults::default();
        for index in pool {
            let Some(deployment) = self.deployments.get(*index) else {
                continue;
            };
            let found = deployment.matching(positive, headers, match_any);
            results.warnings.extend(found.warnings);
            if let Some((via, value)) = found.matched {
                results.matches.push(DeploymentMatch {
                    index: *index,
                    via,
                    value,
                });
            }
        }
        results
    }

    #[must_use]
    pub fn unknown_required_hides_answer(
        &self,
        pool: &[usize],
        excluded: &BTreeSet<String>,
        required: &BTreeSet<String>,
        confirmed: &BTreeSet<String>,
    ) -> bool {
        let mut known = known_tag_values(&self.deployments, pool);
        known.extend(confirmed.iter().cloned());
        let known_required: BTreeSet<String> = required.intersection(&known).cloned().collect();
        if known_required.is_empty() || known_required == *required {
            return false;
        }
        let allowed = exclude(&self.deployments, pool, excluded);
        !require_all(&self.deployments, &allowed, &known_required).is_empty()
    }
}

fn inherited_sets(
    inherited: Option<&[String]>,
    prefix: &str,
) -> (Option<BTreeSet<String>>, Option<BTreeSet<String>>) {
    let Some(tags) = inherited else {
        return (None, None);
    };
    let (rewritten, _) = strip_routing_prefix(tags, prefix);
    let parts = split_tags(&rewritten);
    (
        Some(parts.required.into_iter().collect()),
        Some(parts.excluded.into_iter().collect()),
    )
}
