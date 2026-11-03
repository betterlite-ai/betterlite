use std::collections::BTreeMap;

use crate::sections::content_filter_records::{
    ContentFilterAction, category_types::CategoryFileData,
};

#[derive(Debug)]
pub struct CompiledPhrasePattern {
    pub source: String,
    pub regex: fancy_regex::Regex,
}

#[derive(Debug)]
pub struct CategoryConfig {
    pub category_name: String,
    pub description: String,
    pub default_action: ContentFilterAction,
    pub keywords: Vec<BTreeMap<String, String>>,
    pub exceptions: Vec<String>,
    pub identifier_words: Vec<String>,
    pub always_block_keywords: Vec<BTreeMap<String, String>>,
    pub inherit_from: Option<String>,
    pub additional_block_words: Vec<String>,
    pub phrase_patterns: Vec<CompiledPhrasePattern>,
}

impl CategoryConfig {
    #[must_use]
    pub fn from_data(data: CategoryFileData) -> Self {
        let category_name = data.category_name.unwrap_or_else(|| "unknown".to_owned());
        let default_action = parse_action(data.default_action.as_deref(), &category_name);
        let phrase_patterns = compile_patterns(data.phrase_patterns, &category_name);
        Self {
            category_name,
            description: data.description.unwrap_or_default(),
            default_action,
            keywords: data.keywords,
            exceptions: data
                .exceptions
                .into_iter()
                .map(|word| word.to_lowercase())
                .collect(),
            identifier_words: data
                .identifier_words
                .into_iter()
                .map(|word| word.to_lowercase())
                .collect(),
            always_block_keywords: data.always_block_keywords,
            inherit_from: data.inherit_from,
            additional_block_words: data
                .additional_block_words
                .into_iter()
                .map(|word| word.to_lowercase())
                .collect(),
            phrase_patterns,
        }
    }
}

fn parse_action(action: Option<&str>, category: &str) -> ContentFilterAction {
    match action.unwrap_or("BLOCK") {
        "BLOCK" => ContentFilterAction::Block,
        "MASK" => ContentFilterAction::Mask,
        unknown => {
            tracing::warn!(
                category,
                action = unknown,
                "unknown category action; using BLOCK"
            );
            ContentFilterAction::Block
        }
    }
}

fn compile_patterns(patterns: Vec<String>, category: &str) -> Vec<CompiledPhrasePattern> {
    patterns.into_iter().filter_map(|source| match fancy_regex::Regex::new(&source) {
        Ok(regex) => Some(CompiledPhrasePattern { source, regex }),
        Err(error) => {
            tracing::warn!(category, pattern = source, %error, "invalid category phrase pattern");
            None
        }
    }).collect()
}
