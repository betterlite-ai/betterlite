use betterlite_config::sections::bedrock_services_search::DEFAULT_SEARCH_TOOL_NAME;
use betterlite_models::decisions::Json;
use betterlite_openai::search::provider::SearchQuery;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::SearchError;

pub const AGENTCORE_MAX_QUERY_LENGTH: usize = 200;
pub const AGENTCORE_DEFAULT_MAX_RESULTS: i64 = 10;
pub const AGENTCORE_TOOL_NAME_SUFFIX: &str = "___WebSearch";

pub type Params = IndexMap<String, Json>;

#[must_use]
pub fn transform_search_request(
    query: &SearchQuery,
    optional_params: &Params,
    configured_tool_name: Option<&str>,
) -> Result<Json, SearchError> {
    let joined = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(items) => items.join(" "),
    };
    let tool_name = selected_tool_name(optional_params, configured_tool_name)?;
    if !tool_name.ends_with(AGENTCORE_TOOL_NAME_SUFFIX) {
        return Err(SearchError::ToolName { tool_name });
    }
    let mut arguments = Params::new();
    arguments.insert(
        "query".to_owned(),
        Json::Text(joined.chars().take(AGENTCORE_MAX_QUERY_LENGTH).collect()),
    );
    arguments.insert(
        "maxResults".to_owned(),
        optional_params.get("max_results").map_or(
            Json::Number(Number::from(AGENTCORE_DEFAULT_MAX_RESULTS)),
            Json::clone,
        ),
    );
    let mut params = Params::new();
    params.insert("name".to_owned(), Json::Text(tool_name));
    params.insert("arguments".to_owned(), Json::Object(arguments));
    let mut body = Params::new();
    body.insert("jsonrpc".to_owned(), Json::Text("2.0".to_owned()));
    body.insert("id".to_owned(), Json::Number(Number::from(1_i64)));
    body.insert("method".to_owned(), Json::Text("tools/call".to_owned()));
    body.insert("params".to_owned(), Json::Object(params));
    Ok(Json::Object(body))
}

// LiteLLM takes the first truthy tool_name: the request param, then the configured name, then the
// connector default; only names ending in ___WebSearch may reach the gateway
fn selected_tool_name(
    optional_params: &Params,
    configured: Option<&str>,
) -> Result<String, SearchError> {
    let explicit = match optional_params.get("tool_name") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) if text.is_empty() => None,
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) => return Err(SearchError::WrongType { field: "tool_name" }),
    };
    let selected = explicit
        .or_else(|| configured.filter(|value| !value.is_empty()))
        .unwrap_or(DEFAULT_SEARCH_TOOL_NAME);
    Ok(selected.to_owned())
}
