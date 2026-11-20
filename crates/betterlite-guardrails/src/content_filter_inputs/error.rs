use std::error::Error;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use crate::content_filter_actions::error::ContentFilterBlockDetail::{
    BlockedWord, CategoryKeyword, Conditional, Pattern,
};
use crate::content_filter_actions::error::{ContentFilterActionError, ContentFilterBlockDetail};
use crate::guardrail_base::failure::GuardrailFailure;

pub const BLOCKED_STATUS: u16 = 400;

type BlockDetail = ContentFilterBlockDetail;
type DetailMap = IndexMap<String, GuardrailValue>;

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
        match &self {
            Self::Blocked { detail } => block_failure(detail, None),
            Self::Depth { .. } | Self::Label { .. } => text_failure(self.to_string()),
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
        detail: Box<ContentFilterBlockDetail>,
        text: String,
    },
    #[error(transparent)]
    Filter(ContentFilterActionError),
}

impl<E: Error> ImageProcessingError<E> {
    #[must_use]
    pub fn into_failure(self) -> GuardrailFailure {
        match &self {
            Self::Blocked { detail, text } => block_failure(detail, Some(text.as_str())),
            Self::Completion { .. } | Self::Filter(_) | Self::NoChoices => self.other_failure(),
        }
    }

    fn other_failure(&self) -> GuardrailFailure {
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
pub(super) fn block_detail_map(detail: &BlockDetail, description: Option<&str>) -> DetailMap {
    let mut map = match detail {
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
            description: blocked,
        } => blocked_word_map(error, keyword, blocked.as_deref()),
    };
    if let Some(image_description) = description {
        let error = image_description_error(&map, image_description);
        map.insert("error".to_owned(), GuardrailValue::Text(error));
    }
    map
}

fn image_description_error(map: &DetailMap, description: &str) -> String {
    if let Some(GuardrailValue::Text(error)) = map.get("error") {
        return format!("{error} (Image description): {description}");
    }
    format!("Content blocked: Image description detected{description}")
}

fn blocked_word_map(error: &str, keyword: &str, description: Option<&str>) -> DetailMap {
    let mut map = detail_map_entries(&[("error", error), ("keyword", keyword)]);
    map.insert(
        "description".to_owned(),
        description.map_or(GuardrailValue::Null, |text| {
            GuardrailValue::Text(text.to_owned())
        }),
    );
    map
}

fn block_failure(detail: &BlockDetail, description: Option<&str>) -> GuardrailFailure {
    GuardrailFailure::HttpObject {
        status: BLOCKED_STATUS,
        detail: block_detail_map(detail, description),
    }
}

fn text_failure(error: String) -> GuardrailFailure {
    GuardrailFailure::HttpObject {
        status: BLOCKED_STATUS,
        detail: DetailMap::from([("error".to_owned(), GuardrailValue::Text(error))]),
    }
}

fn detail_map_entries(entries: &[(&str, &str)]) -> DetailMap {
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
