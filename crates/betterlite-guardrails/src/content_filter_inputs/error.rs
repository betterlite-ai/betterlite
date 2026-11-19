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
        match self {
            Self::Blocked { detail } => GuardrailFailure::HttpObject {
                status: BLOCKED_STATUS,
                detail: block_detail_map(&detail),
            },
            Self::Depth { .. } | Self::Label { .. } => GuardrailFailure::HttpObject {
                status: BLOCKED_STATUS,
                detail: IndexMap::from([(
                    "error".to_owned(),
                    GuardrailValue::Text(self.to_string()),
                )]),
            },
            Self::Filter(source) => GuardrailFailure::Other(source.to_string()),
            Self::Serialize { source } => GuardrailFailure::Other(source.to_string()),
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
        detail: ContentFilterBlockDetail,
        description: String,
    },
    #[error(transparent)]
    Filter(ContentFilterActionError),
}

impl<E> ImageProcessingError<E>
where
    E: std::error::Error,
{
    #[must_use]
    pub fn into_failure(self) -> GuardrailFailure {
        if let Self::Blocked {
            detail,
            description,
        } = &self
        {
            return GuardrailFailure::HttpObject {
                status: BLOCKED_STATUS,
                detail: image_block_detail(detail, description),
            };
        }
        let message = match &self {
            Self::NoChoices => self.to_string(),
            Self::Completion { source } => source.to_string(),
            Self::Filter(source) => source.to_string(),
            Self::Blocked { .. } => self.to_string(),
        };
        GuardrailFailure::Other(message)
    }
}

// The HTTP detail LiteLLM raises, with its keys in the source's order
pub(super) fn block_detail_map(
    detail: &ContentFilterBlockDetail,
) -> IndexMap<String, GuardrailValue> {
    match detail {
        Conditional {
            error,
            category,
            matched_phrase,
            severity,
        } => detail_map_entries(&[
            ("error", error),
            ("category", category),
            ("matched_phrase", matched_phrase),
            ("severity", severity),
        ]),
        CategoryKeyword {
            error,
            category,
            keyword,
            severity,
        } => detail_map_entries(&[
            ("error", error),
            ("category", category),
            ("keyword", keyword),
            ("severity", severity),
        ]),
        Pattern { error, pattern } => detail_map_entries(&[("error", error), ("pattern", pattern)]),
        BlockedWord {
            error,
            keyword,
            description,
        } => {
            let mut map = detail_map_entries(&[("error", error), ("keyword", keyword)]);
            map.insert(
                "description".to_owned(),
                description
                    .clone()
                    .map_or(GuardrailValue::Null, GuardrailValue::Text),
            );
            map
        }
    }
}

fn detail_map_entries(entries: &[(&str, &String)]) -> IndexMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), GuardrailValue::Text((*value).clone())))
        .collect()
}

// The error a blocked image description carries keeps the raw description for debugging
pub(super) fn image_block_detail(
    detail: &ContentFilterBlockDetail,
    description: &str,
) -> IndexMap<String, GuardrailValue> {
    let mut map = block_detail_map(detail);
    let appended = if let Some(GuardrailValue::Text(error)) = map.get("error") {
        format!("{error} (Image description): {description}")
    } else {
        format!("Content blocked: Image description detected{description}")
    };
    map.insert("error".to_owned(), GuardrailValue::Text(appended));
    map
}
