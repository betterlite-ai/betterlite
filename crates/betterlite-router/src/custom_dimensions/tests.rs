#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::custom_dimensions as section;

use super::{DimensionsError, validate};

mod custom_dimensions_behaviors;
mod custom_dimensions_boundaries;

pub(super) fn config(
    name: &str,
    weight: f64,
    keywords: &[&str],
    patterns: &[&str],
    scoring_mode: &str,
) -> section::Config {
    section::Config {
        name: name.to_owned(),
        weight,
        keywords: keywords.iter().map(|value| (*value).to_owned()).collect(),
        patterns: patterns.iter().map(|value| (*value).to_owned()).collect(),
        scoring_mode: scoring_mode.to_owned(),
    }
}

pub(super) fn dim(name: &str, keywords: &[&str], patterns: &[&str]) -> section::Config {
    config(name, 0.5, keywords, patterns, "binary")
}

pub(super) fn weighted(weight: f64) -> section::Config {
    config("domain", weight, &["alpha"], &[], "binary")
}

pub(super) fn named(name: &str) -> section::Config {
    config(name, 0.5, &["alpha"], &[], "binary")
}

pub(super) fn counted(matchers: usize) -> section::Config {
    section::Config {
        name: "domain".to_owned(),
        weight: 0.5,
        keywords: (0..matchers).map(|index| format!("k{index}")).collect(),
        patterns: Vec::new(),
        scoring_mode: "binary".to_owned(),
    }
}

pub(super) fn filled(name: &str, lengths: Vec<usize>) -> section::Config {
    section::Config {
        name: name.to_owned(),
        weight: 0.5,
        keywords: Vec::new(),
        patterns: lengths
            .into_iter()
            .map(|length| "a".repeat(length))
            .collect(),
        scoring_mode: "binary".to_owned(),
    }
}

pub(super) fn long_name(length: usize) -> section::Config {
    let name = "a".repeat(length);
    named(&name)
}

pub(super) fn keyworded(length: usize) -> section::Config {
    let keyword = "a".repeat(length);
    config("domain", 0.5, &[keyword.as_str()], &[], "binary")
}

pub(super) fn class_repeat(items: usize) -> section::Config {
    let alphabet: String = "abcdefghijklmnopqrstuvwxyz0123456789"
        .chars()
        .take(items)
        .collect();
    let pattern = format!("[{alphabet}]{{64}}");
    config("domain", 0.5, &[], &[pattern.as_str()], "binary")
}

pub(super) fn many(count: usize) -> Vec<section::Config> {
    (0..count)
        .map(|index| named(&format!("d{index}")))
        .collect()
}

pub(super) fn weights(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs
        .iter()
        .map(|(name, weight)| ((*name).to_owned(), *weight))
        .collect()
}

pub(super) fn run(
    declared: &[section::Config],
    classifier: &str,
    dimension_weights: &BTreeMap<String, f64>,
) -> Result<u64, DimensionsError> {
    validate(declared, classifier, dimension_weights).map(|validated| validated.work)
}
