use std::error::Error;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use crate::content_filter_actions::error::ContentFilterBlockDetail::{
    BlockedWord, CategoryKeyword, Conditional, Pattern,
};
use crate::content_filter_actions::error::{ContentFilterActionError, ContentFilterBlockDetail};
use crate::guardrail_base::failure::GuardrailFailure;

pub const BLOCKED_STATUS: u16 = 400;

#[derive(Debug, thiserror::Error)]
pub enum ArgumentFilterError {
    #[error("the content filter blocked the text")]
    Blocked { detail: ContentFilterBlockDetail },
    #[error(transparent)]
    Filter(ContentFilterActionError),
    #[error("Content blocked: {context_label} arguments exceed the maximum nesting depth")]
    Depth { context_label: &'static str },
    #[error(
        "Content blocked: {context_label} argument matched a masking rule on a non-rewritable field"
    )]
    Label { context_label: &'static str },
    #[error("could not write the filtered arguments as JSON: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}

impl ArgumentFilterError {
    #[must_use]
    pub fn into_failure(self) -> GuardrailFailure {
        let detail = match &self {
            Self::Blocked { detail } => block_detail_map(detail, None),
            Self::Depth { .. } | Self::Label { .. } => {
                IndexMap::from([("error".to_owned(), GuardrailValue::Text(self.to_string()))])
            }
            Self::Filter(source) => return GuardrailFailure::Other(source.to_string()),
            Self::Serialize { source } => return GuardrailFailure::Other(source.to_string()),
        };
        GuardrailFailure::HttpObject {
            status: BLOCKED_STATUS,
            detail,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImageProcessingError<E> {
    #[error("the image description call failed: {source}")]
    Completion {
        #[source]
        source: E,
    },
    #[error("the image description response has no choices")]
    NoChoices,
    #[error("the content filter blocked an image description")]
    Blocked {
        detail: Box<ContentFilterBlockDetail>,
        description: String,
    },
    #[error(transparent)]
    Filter(ContentFilterActionError),
}

impl<E: Error> ImageProcessingError<E> {
    #[must_use]
    pub fn into_failure(self) -> GuardrailFailure {
        if let Self::Blocked {
            detail,
            description,
        } = &self
        {
            return GuardrailFailure::HttpObject {
                status: BLOCKED_STATUS,
                detail: block_detail_map(detail.as_ref(), Some(description.as_str())),
            };
        }
        GuardrailFailure::Other(self.other_message())
    }

    fn other_message(&self) -> String {
        match self {
            Self::Completion { source } => source.to_string(),
            Self::Filter(source) => source.to_string(),
            Self::NoChoices | Self::Blocked { .. } => self.to_string(),
        }
    }
}

// The HTTP detail LiteLLM raises, in its source's order, plus a blocked image's raw description
pub(super) fn block_detail_map(
    detail: &ContentFilterBlockDetail,
    description: Option<&str>,
) -> IndexMap<String, GuardrailValue> {
    let mut map = detail_map(detail);
    if let Some(image_description) = description {
        let appended = match map.get("error") {
            Some(GuardrailValue::Text(error)) => {
                format!("{error} (Image description): {image_description}")
            }
            Some(_) | None => {
                format!("Content blocked: Image description detected{image_description}")
            }
        };
        map.insert("error".to_owned(), GuardrailValue::Text(appended));
    }
    map
}

fn detail_map(detail: &ContentFilterBlockDetail) -> IndexMap<String, GuardrailValue> {
    match detail {
        Conditional {
            error,
            category,
            matched_phrase,
            severity,
        } => detail_map_entries(&[
            ("error", error.as_str()),
            ("category", category.as_str()),
            ("matched_phrase", matched_phrase.as_str()),
            ("severity", severity.as_str()),
        ]),
        CategoryKeyword {
            error,
            category,
            keyword,
            severity,
        } => detail_map_entries(&[
            ("error", error.as_str()),
            ("category", category.as_str()),
            ("keyword", keyword.as_str()),
            ("severity", severity.as_str()),
        ]),
        Pattern { error, pattern } => {
            detail_map_entries(&[("error", error.as_str()), ("pattern", pattern.as_str())])
        }
        BlockedWord {
            error,
            keyword,
            description,
        } => blocked_word_map(error, keyword, description.as_deref()),
    }
}

fn blocked_word_map(
    error: &str,
    keyword: &str,
    description: Option<&str>,
) -> IndexMap<String, GuardrailValue> {
    let mut map = detail_map_entries(&[("error", error), ("keyword", keyword)]);
    map.insert(
        "description".to_owned(),
        description.map_or(GuardrailValue::Null, |value| {
            GuardrailValue::Text(value.to_owned())
        }),
    );
    map
}

fn detail_map_entries(entries: &[(&str, &str)]) -> IndexMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                GuardrailValue::Text((*value).to_owned()),
            )
        })
        .collect()
}
