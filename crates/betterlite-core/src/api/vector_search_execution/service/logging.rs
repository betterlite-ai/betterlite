use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::super::config::{Params, SearchQuery};
use super::super::error::SearchError;
use crate::api::vector_store_http_write::service::LogContext;

// The pre-call carries the transformed body and the signed headers; the base URL is the one before the search path
pub(super) fn log_http(
    api_base: &str,
    headers: &OpenAiHeaders,
    body: &Params,
    log: &mut LogContext<'_, '_>,
) -> Result<(), SearchError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.expose_secret().to_owned()));
    }
    let complete = body
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: Some(RawBody::Object(complete)),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env)
        .map(drop)
        .map_err(SearchError::Logging)
}

// A direct store has no HTTP request to echo; the synthetic endpoint is what keeps the logger from dumping the stored credentials
pub(super) fn log_direct(
    provider: &str,
    vector_store_id: &str,
    query: &SearchQuery,
    log: &mut LogContext<'_, '_>,
) -> Result<(), SearchError> {
    let endpoint = format!("{provider}://{vector_store_id}");
    let mut passthrough = IndexMap::new();
    passthrough.insert("query".to_owned(), query_json(query));
    passthrough.insert(
        "vector_store_id".to_owned(),
        Json::Text(vector_store_id.to_owned()),
    );
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(endpoint.clone()),
            headers: None,
            complete_input_dict: None,
            request_str: Some(format!("direct vector store search: {endpoint}")),
            openai_sdk: None,
            passthrough,
        },
    };
    pre_call(log.state, &record, log.env)
        .map(drop)
        .map_err(SearchError::Logging)
}

fn query_json(query: &SearchQuery) -> Json {
    match query {
        SearchQuery::Text(value) => Json::Text(value.clone()),
        SearchQuery::Sequence(parts) => {
            Json::List(parts.iter().map(|part| Json::Text(part.clone())).collect())
        }
    }
}
