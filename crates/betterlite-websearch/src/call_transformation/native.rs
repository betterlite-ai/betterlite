use betterlite_error::error::ErrorClass;
use betterlite_openai::search::records::SearchResponse;
use serde::Serialize;
use serde_json::value::to_raw_value;

use super::TransformationError;
use super::arguments::{Budget, quote, text};
use crate::records::{SearchOutcome, WebSearchToolResultErrorCode};

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResultBlock {
    WebSearchToolResult {
        tool_use_id: String,
        content: ResultContent,
    },
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResultContent {
    Items(Vec<ResultItem>),
    Error(ResultError),
}
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResultItem {
    WebSearchResult {
        url: String,
        title: String,
        page_age: Option<String>,
        encrypted_content: String,
        snippet: String,
    },
}
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResultError {
    WebSearchToolResultError {
        error_code: WebSearchToolResultErrorCode,
    },
}

#[must_use]
pub fn result_block(id: &str, response: Option<&SearchResponse>) -> ResultBlock {
    let items = response.map_or_else(Vec::new, |search| {
        search
            .results
            .iter()
            .map(|result| ResultItem::WebSearchResult {
                url: result.url.clone(),
                title: result.title.clone(),
                snippet: result.snippet.clone(),
                page_age: result
                    .date
                    .as_ref()
                    .filter(|date| !date.is_empty())
                    .or(result.last_updated.as_ref())
                    .cloned(),
                encrypted_content: String::new(),
            })
            .collect()
    });
    ResultBlock::WebSearchToolResult {
        tool_use_id: id.to_owned(),
        content: ResultContent::Items(items),
    }
}

#[must_use]
pub fn result_error_block(id: &str, code: WebSearchToolResultErrorCode) -> ResultBlock {
    let content = ResultContent::Error(ResultError::WebSearchToolResultError { error_code: code });
    ResultBlock::WebSearchToolResult {
        tool_use_id: id.to_owned(),
        content,
    }
}

#[must_use]
pub fn outcome_block(id: &str, outcome: &SearchOutcome) -> ResultBlock {
    match outcome {
        SearchOutcome::Succeeded(success) => result_block(id, success.response.as_ref()),
        SearchOutcome::Failed(failure) => result_error_block(id, failure.error_code),
    }
}

#[must_use]
pub const fn search_error_code(class: &ErrorClass) -> WebSearchToolResultErrorCode {
    match class {
        ErrorClass::RateLimit(_) => WebSearchToolResultErrorCode::TooManyRequests,
        // BadRequestError's Python subclasses also match `case BadRequestError():` by isinstance.
        ErrorClass::BadRequest(_)
        | ErrorClass::ContextWindowExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::UnsupportedParams(_) => WebSearchToolResultErrorCode::InvalidToolInput,
        ErrorClass::Authentication(_)
        | ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::Timeout(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ServiceUnavailable(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::Api(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => WebSearchToolResultErrorCode::Unavailable,
    }
}

pub fn format_search_response(response: &SearchResponse) -> Result<String, TransformationError> {
    if response.results.is_empty() {
        let mut budget = Budget::default();
        let mut fields = vec![format!(
            "results=[] object={}",
            quote(&response.object.0, true)?
        )];
        for (key, value) in &response.extra {
            let raw = to_raw_value(value)?;
            fields.push(format!("{key}={}", text(&raw, true, &mut budget)?));
        }
        return Ok(fields.join(" "));
    }
    Ok(response
        .results
        .iter()
        .map(|result| {
            format!(
                "Title: {}\nURL: {}\nSnippet: {}",
                result.title, result.url, result.snippet
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}
