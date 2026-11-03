use betterlite_config::sections::content_filter_catalog::patterns::{
    PatternCatalogError, get_compiled_pattern, pattern_extra_config,
};
use betterlite_config::sections::content_filter_records::{
    ContentFilterAction, ContentFilterPattern,
};
use fancy_regex::Regex;

pub mod blocked_words;
pub mod masking;
mod python_regex;
pub mod spans;
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum PatternError {
    #[error("pattern_name is required for prebuilt patterns")]
    MissingName,
    #[error("pattern is required for regex patterns")]
    MissingPattern,
    #[error("Unknown pattern_type: {0}")]
    UnknownType(String),
    #[error(transparent)]
    Catalog(#[from] PatternCatalogError),
    #[error("invalid regex or matching failed: {0}")]
    Regex(#[from] fancy_regex::Error),
    #[error("text exceeds the pattern matching size limit")]
    TooLarge,
    #[error("invalid text span")]
    InvalidSpan,
}

#[derive(Debug)]
pub struct CompiledPattern {
    pub regex: Regex,
    pub pattern_name: String,
    pub action: ContentFilterAction,
    pub keyword_regex: Option<Regex>,
    pub allow_word_numbers: bool,
}

pub fn compile(config: &ContentFilterPattern) -> Result<CompiledPattern, PatternError> {
    match config.pattern_type.as_str() {
        "prebuilt" => compile_prebuilt(config),
        "regex" => {
            let source = config
                .pattern
                .as_ref()
                .ok_or(PatternError::MissingPattern)?;
            if source.len() > spans::MAX_TEXT_BYTES {
                return Err(PatternError::TooLarge);
            }
            Ok(CompiledPattern {
                regex: python_regex::compile(source)?,
                pattern_name: config
                    .name
                    .clone()
                    .unwrap_or_else(|| "custom_regex".to_owned()),
                action: config.action,
                keyword_regex: None,
                allow_word_numbers: false,
            })
        }
        other => Err(PatternError::UnknownType(other.to_owned())),
    }
}

fn compile_prebuilt(config: &ContentFilterPattern) -> Result<CompiledPattern, PatternError> {
    let name = config
        .pattern_name
        .as_ref()
        .ok_or(PatternError::MissingName)?;
    let regex = get_compiled_pattern(name)?;
    let extras = pattern_extra_config()?;
    let extra = extras.get(name);
    let keyword = extra.and_then(|value| value.keyword_pattern.as_deref());
    Ok(CompiledPattern {
        regex,
        pattern_name: name.clone(),
        action: config.action,
        keyword_regex: keyword.map(python_regex::compile).transpose()?,
        allow_word_numbers: extra
            .and_then(|value| value.allow_word_numbers)
            .unwrap_or(false),
    })
}

pub fn check_patterns<'text, 'pattern>(
    text: &'text str,
    patterns: &'pattern [CompiledPattern],
) -> Result<Option<(&'text str, &'pattern str, ContentFilterAction)>, PatternError> {
    for pattern in patterns {
        if let Some(span) = pattern.find_spans(text)?.first() {
            let matched = text.get(span.clone()).ok_or(PatternError::InvalidSpan)?;
            return Ok(Some((matched, &pattern.pattern_name, pattern.action)));
        }
    }
    Ok(None)
}
