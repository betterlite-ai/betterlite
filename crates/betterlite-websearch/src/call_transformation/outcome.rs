use betterlite_error::error::ErrorClass;
use betterlite_openai::search::records::SearchResponse;

use super::native::search_error_code;
use crate::records::{SearchFailed, SearchOutcome, SearchSucceeded};

// ErrorClass has no generic message accessor across its families, so the message travels alongside it.
#[derive(Debug, Clone)]
pub struct SearchFailure {
    pub class: ErrorClass,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum SearchResult {
    Failed(SearchFailure),
    Succeeded {
        text: String,
        response: Option<SearchResponse>,
    },
    Unexpected(String),
}

#[must_use]
pub fn search_outcome(result: SearchResult) -> SearchOutcome {
    match result {
        SearchResult::Failed(failure) => {
            tracing::error!(
                "WebSearchInterception: Search failed with error: {}",
                failure.message
            );
            SearchOutcome::Failed(SearchFailed {
                error_code: search_error_code(&failure.class),
                message: failure.message,
            })
        }
        SearchResult::Succeeded { text, response } => {
            SearchOutcome::Succeeded(SearchSucceeded { text, response })
        }
        SearchResult::Unexpected(text) => {
            tracing::debug!("WebSearchInterception: Unexpected search result type");
            SearchOutcome::Succeeded(SearchSucceeded {
                text,
                response: None,
            })
        }
    }
}

#[must_use]
pub fn search_outcome_text(outcome: &SearchOutcome) -> String {
    match outcome {
        SearchOutcome::Succeeded(success) => success.text.clone(),
        SearchOutcome::Failed(failure) => format!("Search failed: {}", failure.message),
    }
}
