use serde::{Deserialize, Serialize};

use super::error::GuardrailParamsError;
use crate::sections::content_filter_records::{
    BlockedWord, ContentFilterCategoryConfig, ContentFilterPattern,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Vec<ContentFilterPattern>")]
pub struct CheckedPatterns(pub Vec<ContentFilterPattern>);

impl TryFrom<Vec<ContentFilterPattern>> for CheckedPatterns {
    type Error = GuardrailParamsError;

    fn try_from(patterns: Vec<ContentFilterPattern>) -> Result<Self, Self::Error> {
        if patterns
            .iter()
            .any(|pattern| !matches!(pattern.pattern_type.as_str(), "prebuilt" | "regex"))
        {
            return Err(GuardrailParamsError::InvalidField {
                field: "pattern_type".to_owned(),
            });
        }
        Ok(Self(patterns))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentFilterConfigModel {
    pub patterns: Option<CheckedPatterns>,
    pub blocked_words: Option<Vec<BlockedWord>>,
    pub blocked_words_file: Option<String>,
    pub categories: Option<Vec<ContentFilterCategoryConfig>>,
    pub severity_threshold: Option<String>,
    pub pattern_redaction_format: Option<String>,
    pub keyword_redaction_tag: Option<String>,
}
