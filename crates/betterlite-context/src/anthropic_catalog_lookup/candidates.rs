use regex::Regex;

use super::error::CatalogLookupError;
use crate::bedrock_model::regions::CROSS_REGION_PREFIXES;
use crate::bedrock_model::{
    extract_model_from_arn, get_bedrock_base_model, strip_bedrock_routing_prefix,
    strip_throughput_suffix,
};

const ROUTING_PREFIXES: [&str; 4] = [
    "bedrock/converse/",
    "bedrock/invoke/",
    "bedrock/",
    "vertex_ai/",
];

// Each suffix pattern keeps an optional trailing newline, because Python's `$` also matches just before one.
pub struct CandidatePatterns {
    version: Regex,
    minor: Regex,
    dated: Regex,
    dotted: Regex,
}

impl CandidatePatterns {
    pub fn new() -> Result<Self, CatalogLookupError> {
        Ok(Self {
            version: Regex::new(r"-v\d+(?::\d+)?(\n?)\z")?,
            minor: Regex::new(r":\d+(\n?)\z")?,
            dated: Regex::new(r"-\d{8}(\n?)\z")?,
            dotted: Regex::new(r"(\d)\.(\d)")?,
        })
    }

    #[must_use]
    pub fn candidates(&self, model: &str) -> Vec<String> {
        let mut primary = vec![model.to_owned()];
        primary.extend(
            ROUTING_PREFIXES
                .iter()
                .filter_map(|prefix| model.strip_prefix(prefix))
                .map(str::to_owned),
        );
        if let Some(base) = bedrock_base(model) {
            primary.push(base.clone());
            primary.push(format!("bedrock/{base}"));
        }
        let mut candidates: Vec<String> = Vec::new();
        let normalized: Vec<String> = primary
            .iter()
            .flat_map(|candidate| self.variants(candidate))
            .collect();
        for candidate in primary.into_iter().chain(normalized) {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        candidates
    }

    fn variants(&self, candidate: &str) -> [String; 6] {
        let dotted = self.dotted.replace_all(candidate, "${1}-${2}");
        let versionless = self.version.replace(&dotted, "$1");
        let composite = self.dated.replace(&versionless, "$1").into_owned();
        // LiteLLM cuts at the last "@" only when it is not the first character
        let at_stripped = candidate
            .rfind('@')
            .filter(|at| *at > 0)
            .and_then(|at| candidate.get(..at))
            .unwrap_or(candidate);
        [
            self.version.replace(candidate, "$1").into_owned(),
            self.minor.replace(candidate, "$1").into_owned(),
            self.dated.replace(candidate, "$1").into_owned(),
            dotted.into_owned(),
            composite,
            at_stripped.to_owned(),
        ]
    }
}

// LiteLLM swallows the IndexError a bare cross-region prefix raises, which B2203 returns unchanged instead.
fn bedrock_base(model: &str) -> Option<String> {
    let base = get_bedrock_base_model(model);
    let routed = strip_throughput_suffix(&extract_model_from_arn(&strip_bedrock_routing_prefix(
        model,
    )));
    let raised = base == routed && CROSS_REGION_PREFIXES.contains(&base.as_str());
    (!base.is_empty() && !raised).then_some(base)
}
