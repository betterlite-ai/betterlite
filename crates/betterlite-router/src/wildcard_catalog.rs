use std::fmt;

use regex::Regex;

pub mod error;

mod tests;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::wildcard_catalog::error::PatternCatalogError;

const COMPLEXITY_CHARS: [char; 9] = ['*', '+', '?', '\\', '^', '$', '|', '(', ')'];
// Python 3.7 escapes exactly these characters, and LiteLLM sorts patterns by the escaped length
const PYTHON_ESCAPED_CHARS: [char; 24] = [
    '(', ')', '[', ']', '{', '}', '?', '*', '+', '-', '|', '^', '$', '\\', '.', '&', '~', '#', ' ',
    '\t', '\n', '\r', '\u{b}', '\u{c}',
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub model_id: Option<String>,
    pub model: String,
}

// A candidate as registered, beside the deployment row it came from, so a reordered rebuild cannot move it.
#[derive(Clone)]
struct Registered {
    candidate: Candidate,
    deployment: Option<DeploymentSpec>,
}

// The candidate alone: the registered deployment carries credentials, and Debug never prints them.
impl fmt::Debug for Registered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registered")
            .field("candidate", &self.candidate)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
pub struct PatternCatalog {
    patterns: Vec<Pattern>,
}

#[derive(Clone, Debug)]
struct Pattern {
    key: String,
    matcher: Regex,
    candidates: Vec<Registered>,
    specificity: (usize, usize),
}

impl PatternCatalog {
    pub fn add_pattern(
        &mut self,
        pattern: &str,
        candidate: Candidate,
    ) -> Result<(), PatternCatalogError> {
        let key = compiled_pattern(pattern);
        if let Some(existing) = self.patterns.iter_mut().find(|entry| entry.key == key) {
            existing.candidates.push(Registered {
                candidate,
                deployment: None,
            });
            return Ok(());
        }
        let matcher = Regex::new(&matcher_source(pattern)).map_err(|source| {
            PatternCatalogError::Compile {
                pattern: pattern.to_owned(),
                source,
            }
        })?;
        let specificity = pattern_specificity(&key);
        let position = self
            .patterns
            .iter()
            .position(|entry| entry.specificity < specificity)
            .unwrap_or(self.patterns.len());
        self.patterns.insert(
            position,
            Pattern {
                key,
                matcher,
                candidates: vec![Registered {
                    candidate,
                    deployment: None,
                }],
                specificity,
            },
        );
        Ok(())
    }

    // Record the deployment a push serves on the pattern's last registration, once, so a rebuild keeps that snapshot.
    pub fn record_deployment(&mut self, pattern: &str, deployment: &DeploymentSpec) {
        let key = compiled_pattern(pattern);
        if let Some(entry) = self.patterns.iter_mut().find(|entry| entry.key == key)
            && let Some(registered) = entry.candidates.last_mut()
            && registered.deployment.is_none()
        {
            registered.deployment = Some(deployment.clone());
        }
    }

    pub fn remove_deployment(&mut self, model_id: &str) {
        for entry in &mut self.patterns {
            entry
                .candidates
                .retain(|registered| registered.candidate.model_id.as_deref() != Some(model_id));
        }
        self.patterns.retain(|entry| !entry.candidates.is_empty());
    }

    #[must_use]
    pub fn route(
        &self,
        request: &str,
        filtered_model_names: Option<&[String]>,
    ) -> Option<Vec<Candidate>> {
        let (entry, segments) = self.matched(request, filtered_model_names)?;
        Some(
            registered_candidates(entry, request, &segments)
                .into_iter()
                .map(|(candidate, _)| candidate)
                .collect(),
        )
    }

    // The first pattern the request matches, beside the text each of its wildcards captured.
    fn matched<'catalog, 'request>(
        &'catalog self,
        request: &'request str,
        filtered_model_names: Option<&[String]>,
    ) -> Option<(&'catalog Pattern, Vec<&'request str>)> {
        let filtered = filtered_model_names.map(|names| {
            names
                .iter()
                .map(|name| compiled_pattern(name))
                .collect::<Vec<_>>()
        });
        let found = self.patterns.iter().find_map(|entry| {
            let selected = filtered
                .as_ref()
                .is_none_or(|keys| keys.contains(&entry.key));
            if !selected {
                return None;
            }
            entry
                .matcher
                .captures(request)
                .map(|captures| (entry, captures))
        });
        let (entry, captures) = found?;
        // every group of a wildcard pattern is (.*), which always captures, even an empty span
        let segments = (1..captures.len())
            .map(|index| captures.get(index).map_or("", |segment| segment.as_str()))
            .collect();
        Some((entry, segments))
    }

    // get_pattern's candidates, each beside the deployment row that registered it, with the provider retry.
    #[must_use]
    pub fn get_pattern_rows(
        &self,
        model: &str,
        provider: Option<&str>,
    ) -> Option<Vec<(Candidate, Option<DeploymentSpec>)>> {
        if let Some((entry, segments)) = self.matched(model, None) {
            return Some(registered_candidates(entry, model, &segments));
        }
        let resolved_provider = provider?;
        let qualified = format!("{resolved_provider}/{model}");
        let (entry, segments) = self.matched(&qualified, None)?;
        Some(registered_candidates(entry, &qualified, &segments))
    }

    // the caller resolves custom_llm_provider or the inferred provider, as LiteLLM does before its fallback
    #[must_use]
    pub fn get_pattern(&self, model: &str, provider: Option<&str>) -> Option<Vec<Candidate>> {
        self.get_pattern_rows(model, provider)
            .map(|rows| rows.into_iter().map(|(candidate, _)| candidate).collect())
    }

    #[must_use]
    pub fn get_deployments_by_pattern(
        &self,
        model: &str,
        provider: Option<&str>,
    ) -> Vec<Candidate> {
        // a pattern miss is the empty deployment list LiteLLM answers with instead of none
        self.get_pattern(model, provider).unwrap_or_default()
    }
}

// LiteLLM keys its pattern map by this string, so the escaping decides both duplicates and ties
fn compiled_pattern(pattern: &str) -> String {
    let mut escaped = String::with_capacity(pattern.len());
    for letter in pattern.chars() {
        if PYTHON_ESCAPED_CHARS.contains(&letter) {
            escaped.push('\\');
        }
        escaped.push(letter);
    }
    escaped.replace("\\*", "(.*)")
}

fn matcher_source(pattern: &str) -> String {
    // Python's re.match anchors only at the start, so a pattern without a star also matches a longer request
    format!("^(?:{})", regex::escape(pattern).replace("\\*", "(.*)"))
}

fn pattern_specificity(key: &str) -> (usize, usize) {
    let length = key.chars().count();
    let complexity = key
        .chars()
        .filter(|letter| COMPLEXITY_CHARS.contains(letter))
        .count();
    (length, complexity)
}

// One pattern's candidates for the request, each beside the deployment row that registered it.
fn registered_candidates(
    entry: &Pattern,
    request: &str,
    segments: &[&str],
) -> Vec<(Candidate, Option<DeploymentSpec>)> {
    entry
        .candidates
        .iter()
        .map(|registered| {
            (
                Candidate {
                    model_id: registered.candidate.model_id.clone(),
                    model: substitute_model(&registered.candidate.model, request, segments),
                },
                registered.deployment.clone(),
            )
        })
        .collect()
}

fn substitute_model(model: &str, request: &str, segments: &[&str]) -> String {
    if !model.contains('*') {
        return model.to_owned();
    }
    let wildcards = model.matches('*').count();
    if segments.len() > wildcards {
        return request.to_owned();
    }
    let mut resolved = model.to_owned();
    for segment in segments {
        resolved = resolved.replacen('*', segment, 1);
    }
    resolved
}
