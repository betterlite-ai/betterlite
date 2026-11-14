use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_streaming::STREAM_SCAN_CONTEXT_CHARS;

mod tests;

// LiteLLM's _StreamedScanPlan; Python leaves the fields without defaults, Rust starts from empty
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamedScanPlan {
    pub context_chars: usize,
    pub exception_phrases: Vec<String>,
    pub conditional_words: Vec<String>,
}

#[must_use]
pub fn streamed_scan_plan(source: &ContentFilterSource<'_>) -> StreamedScanPlan {
    let categories = source.categories;
    let longest_keyword = source
        .blocked_words
        .iter()
        .map(|word| word.keyword.chars().count())
        .chain(
            categories
                .category_keywords
                .keys()
                .map(|keyword| keyword.chars().count()),
        )
        .chain(
            categories
                .always_block_category_keywords
                .keys()
                .map(|keyword| keyword.chars().count()),
        )
        .max()
        .unwrap_or(0);
    let exception_phrases = categories
        .loaded_categories
        .values()
        .flat_map(|category| category.exceptions.iter().cloned())
        .collect();
    let conditional_words = categories
        .conditional_categories
        .values()
        .flat_map(|config| {
            config
                .identifier_words
                .iter()
                .chain(&config.block_words)
                .cloned()
        })
        .collect();
    StreamedScanPlan {
        context_chars: STREAM_SCAN_CONTEXT_CHARS.max(longest_keyword),
        exception_phrases,
        conditional_words,
    }
}
