use regex::Regex;

pub mod error;

mod tests;

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

#[derive(Clone, Debug, Default)]
pub struct PatternCatalog {
    patterns: Vec<Pattern>,
}

#[derive(Clone, Debug)]
struct Pattern {
    key: String,
    matcher: Regex,
    candidates: Vec<Candidate>,
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
            existing.candidates.push(candidate);
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
                candidates: vec![candidate],
                specificity,
            },
        );
        Ok(())
    }

    pub fn remove_deployment(&mut self, model_id: &str) {
        for entry in &mut self.patterns {
            entry
                .candidates
                .retain(|candidate| candidate.model_id.as_deref() != Some(model_id));
        }
        self.patterns.retain(|entry| !entry.candidates.is_empty());
    }

    #[must_use]
    pub fn route(
        &self,
        request: &str,
        filtered_model_names: Option<&[String]>,
    ) -> Option<Vec<Candidate>> {
        let filtered = filtered_model_names.map(|names| {
            names
                .iter()
                .map(|name| compiled_pattern(name))
                .collect::<Vec<_>>()
        });
        let matched = self.patterns.iter().find_map(|entry| {
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
        let (entry, captures) = matched?;
        // every group of a wildcard pattern is (.*), which always captures, even an empty span
        let segments: Vec<&str> = (1..captures.len())
            .map(|index| captures.get(index).map_or("", |segment| segment.as_str()))
            .collect();
        Some(
            entry
                .candidates
                .iter()
                .map(|candidate| Candidate {
                    model_id: candidate.model_id.clone(),
                    model: substitute_model(&candidate.model, request, &segments),
                })
                .collect(),
        )
    }

    #[must_use]
    pub fn get_pattern(&self, model: &str, provider: Option<&str>) -> Option<Vec<Candidate>> {
        if let Some(candidates) = self.route(model, None) {
            return Some(candidates);
        }
        let resolved_provider = provider?;
        self.route(&format!("{resolved_provider}/{model}"), None)
    }

    #[must_use]
    pub fn get_deployments_by_pattern(
        &self,
        model: &str,
        provider: Option<&str>,
    ) -> Vec<Candidate> {
        let Some(candidates) = self.get_pattern(model, provider) else {
            return Vec::new();
        };
        candidates
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
