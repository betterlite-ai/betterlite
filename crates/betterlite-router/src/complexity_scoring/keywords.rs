use std::collections::BTreeSet;

use regex::{Regex, RegexBuilder};

use crate::complexity_scoring::error::ScoringError;

enum Matcher {
    Substring,
    Word(Regex),
}

struct Keyword {
    original: String,
    lowered: String,
    matcher: Matcher,
}

pub struct KeywordList {
    keywords: Vec<Keyword>,
}

impl KeywordList {
    pub fn compile(keywords: &[String]) -> Result<Self, ScoringError> {
        let mut compiled = Vec::with_capacity(keywords.len());
        for keyword in keywords {
            let lowered = keyword.to_lowercase();
            let matcher = if lowered.contains(' ') || contains_cjk(&lowered) {
                Matcher::Substring
            } else {
                Matcher::Word(Regex::new(&format!(r"\b{}\b", regex::escape(&lowered)))?)
            };
            compiled.push(Keyword {
                original: keyword.clone(),
                lowered,
                matcher,
            });
        }
        Ok(Self { keywords: compiled })
    }

    #[must_use]
    pub fn matches(&self, text: &str) -> Vec<&str> {
        self.keywords
            .iter()
            .filter(|keyword| keyword.hit(text))
            .map(|keyword| keyword.original.as_str())
            .collect()
    }
}

impl Keyword {
    fn hit(&self, text: &str) -> bool {
        match &self.matcher {
            Matcher::Substring => text.contains(self.lowered.as_str()),
            Matcher::Word(pattern) => pattern.is_match(text),
        }
    }
}

// _keyword_matches: a word matches on boundaries so "api" misses "capital", while a phrase or CJK term is a substring
fn contains_cjk(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character,
            '\u{3040}'..='\u{30ff}'
                | '\u{31f0}'..='\u{31ff}'
                | '\u{3400}'..='\u{4dbf}'
                | '\u{4e00}'..='\u{9fff}'
                | '\u{f900}'..='\u{faff}'
                | '\u{ff66}'..='\u{ff9d}'
                | '\u{20000}'..='\u{3ffff}'
        )
    })
}

pub fn compile_patterns(patterns: &[String]) -> Result<Vec<Regex>, ScoringError> {
    let mut compiled = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        compiled.push(Regex::new(pattern)?);
    }
    Ok(compiled)
}

// custom dimension patterns match case-insensitively, as the pinned definition compiles them
pub fn compile_insensitive_patterns(patterns: &[String]) -> Result<Vec<Regex>, ScoringError> {
    let mut compiled = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        compiled.push(RegexBuilder::new(pattern).case_insensitive(true).build()?);
    }
    Ok(compiled)
}

// _append_custom_keywords: base order first, then each new custom term; a repeat keeps its first position and last spelling
pub fn append_keywords(base: &[String], custom: &[String]) -> Vec<String> {
    let mut merged = base.to_vec();
    let base_lowered: BTreeSet<String> =
        base.iter().map(|keyword| keyword.to_lowercase()).collect();
    let custom_start = merged.len();
    for keyword in custom {
        let lowered = keyword.to_lowercase();
        if base_lowered.contains(&lowered) {
            continue;
        }
        let seen = merged[custom_start..]
            .iter()
            .position(|merged_keyword| merged_keyword.to_lowercase() == lowered);
        match seen {
            Some(offset) => merged[custom_start + offset] = keyword.clone(),
            None => merged.push(keyword.clone()),
        }
    }
    merged
}
