use std::error::Error;

use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_openai::search::provider::SearchQuery;
use serde_json::value::to_raw_value;

use crate::call_transformation::outcome::SearchFailure;
use crate::records::RichWebSearchInput;
use crate::search_tool_access::parent_correlation::ParentRequestCorrelation;
use crate::search_tool_access::tool_selection::SearchToolParams;

pub const DEFAULT_SEARCH_PROVIDER: &str = "perplexity";

#[must_use]
pub fn query_argument(
    query: &str,
    rich: Option<&RichWebSearchInput>,
    rich_supported: bool,
) -> SearchQuery {
    if rich_supported
        && let Some(queries) = rich
            .and_then(|input| input.search_queries.as_deref())
            .filter(|queries| !queries.is_empty())
    {
        return SearchQuery::Multiple(queries.to_vec());
    }
    SearchQuery::Single(query.to_owned())
}

pub fn search_params(
    litellm_params: SearchToolParams,
    rich: Option<&RichWebSearchInput>,
    rich_supported: bool,
    correlation: &ParentRequestCorrelation,
) -> Result<SearchToolParams, SearchFailure> {
    // RawValue keeps the source token, and null has a single spelling in JSON
    let mut params: SearchToolParams = litellm_params
        .into_iter()
        .filter(|(_key, value)| value.get() != "null")
        .collect();
    if rich_supported {
        insert_objective(&mut params, rich)?;
    }
    for (key, value) in correlation.as_search_kwargs() {
        let raw = to_raw_value(&value).map_err(|error| unclassified(error.to_string()))?;
        params.insert(key, raw);
    }
    Ok(params)
}

fn insert_objective(
    params: &mut SearchToolParams,
    rich: Option<&RichWebSearchInput>,
) -> Result<(), SearchFailure> {
    let Some(objective) = rich
        .and_then(|input| input.objective.as_deref())
        .filter(|objective| !objective.is_empty())
    else {
        return Ok(());
    };
    if params.contains_key("objective") {
        return Ok(());
    }
    let raw = to_raw_value(&objective).map_err(|error| unclassified(error.to_string()))?;
    params.insert("objective".to_owned(), raw);
    Ok(())
}

pub fn deepest_message(error: &dyn Error) -> String {
    let mut deepest = error;
    while let Some(source) = deepest.source() {
        deepest = source;
    }
    deepest.to_string()
}

#[must_use]
pub fn unclassified(message: String) -> SearchFailure {
    let context = ErrorContext {
        message: message.clone(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    SearchFailure {
        class: ErrorClass::Unclassified(Box::new(context)),
        message,
    }
}
