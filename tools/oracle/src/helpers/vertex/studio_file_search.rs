use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_models::decisions::Json;
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::studio_file_search::RequestParams;
use betterlite_vertex::studio_file_search::config::{capabilities, validate_environment};
use betterlite_vertex::studio_file_search::create::create_request;
use betterlite_vertex::studio_file_search::decode::{
    Decoded, RawResponse, Shape, decode_studio_file_search,
};
use betterlite_vertex::studio_file_search::error::StudioFileSearchError;
use betterlite_vertex::studio_file_search::request::{
    SearchInput, SearchQuery, complete_url, search_request,
};
use betterlite_vertex::studio_identity::Headers;
use jiff::tz::TimeZone;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Capabilities,
    Environment(EnvironmentCall),
    Url(UrlCall),
    Search(SearchCall),
    SearchReply(ReplyCall),
    Create(CreateCall),
    CreateReply(CreateReplyCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    params: Option<EnvironmentParams>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentParams {
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchCall {
    vector_store_id: String,
    query: QueryInput,
    #[serde(default)]
    filters: Option<Json>,
    #[serde(default)]
    max_num_results: Option<i64>,
    #[serde(default)]
    extra_body: Option<Json>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QueryInput {
    Text(String),
    Sequence(Vec<String>),
}

impl QueryInput {
    fn search_query(&self) -> SearchQuery {
        match self {
            Self::Text(text) => SearchQuery::Text(text.clone()),
            Self::Sequence(items) => SearchQuery::Sequence(items.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    body: String,
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateReplyCall {
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    body: String,
    timezone: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let calls = input
            .calls
            .iter()
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call) -> HelperResult {
    match call {
        Call::Capabilities => Ok(capabilities_view()),
        Call::Environment(given) => environment_view(given),
        Call::Url(given) => Ok(url_view(given)),
        Call::Search(given) => search_view(given),
        Call::SearchReply(given) => search_reply_view(given),
        Call::Create(given) => Ok(create_view(given)),
        Call::CreateReply(given) => create_reply_view(given),
    }
}

fn capabilities_view() -> Value {
    let found = capabilities();
    json!({
        "endpoints": found.endpoints,
        "credentials": found.credentials,
        "supported": found.supported_params,
    })
}

fn environment_view(call: &EnvironmentCall) -> HelperResult {
    let settings = settings(&call.environment);
    let params = call.params.as_ref().map(request_params);
    let headers = validate_environment(&call.headers, params.as_ref(), &settings);
    Ok(json!({ "headers": exposed(&headers) }))
}

fn request_params(given: &EnvironmentParams) -> RequestParams {
    RequestParams {
        api_key: given.api_key.clone().map(SecretString::from),
        model: None,
    }
}

fn exposed(headers: &Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

fn url_view(call: &UrlCall) -> Value {
    let settings = settings(&call.environment);
    json!({ "url": complete_url(call.api_base.as_deref(), &settings) })
}

fn search_view(call: &SearchCall) -> HelperResult {
    let settings = settings(&call.environment);
    let params = RequestParams {
        api_key: call.api_key.clone().map(SecretString::from),
        model: call.model.clone(),
    };
    let query = call.query.search_query();
    let plan = search_request(&SearchInput {
        vector_store_id: &call.vector_store_id,
        query: &query,
        filters: call.filters.as_ref(),
        max_num_results: call.max_num_results,
        extra_body: call.extra_body.as_ref(),
        params: &params,
        api_base: call.api_base.as_deref(),
        settings: &settings,
    })
    .map_err(failure)?;
    Ok(json!({ "url": plan.url, "body": plan.body }))
}

fn search_reply_view(call: &ReplyCall) -> HelperResult {
    let local = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let response = RawResponse {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    };
    let decoded = decode_studio_file_search(Shape::Search, &response, &call.query, &local)
        .map_err(failure)?;
    record_value(decoded)
}

fn create_view(call: &CreateCall) -> Value {
    let settings = settings(&call.environment);
    let api_base = complete_url(call.api_base.as_deref(), &settings);
    let plan = create_request(call.name.as_deref(), &api_base);
    json!({ "url": plan.url, "body": plan.body })
}

fn create_reply_view(call: &CreateReplyCall) -> HelperResult {
    let zone = TimeZone::get(&call.timezone).or_value_error()?;
    let local = LocalTimeContext { zone };
    let response = RawResponse {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    };
    let decoded =
        decode_studio_file_search(Shape::Create, &response, "", &local).map_err(failure)?;
    record_value(decoded)
}

fn record_value(decoded: Decoded) -> HelperResult {
    match decoded {
        Decoded::Search(record) => Ok(serde_json::to_value(&*record)?),
        Decoded::Create(record) => Ok(serde_json::to_value(&*record)?),
    }
}

fn failure(error: StudioFileSearchError) -> Box<HelperError> {
    let message = error.to_string();
    match error {
        StudioFileSearchError::SearchReply { status, .. }
        | StudioFileSearchError::CreateReply { status, .. } => Box::new(HelperError {
            class: "GeminiError".to_owned(),
            status: Some(status),
            error_type: None,
            param: None,
            code: None,
            message,
        }),
        StudioFileSearchError::MissingKey | StudioFileSearchError::Json { .. } => {
            Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        }
    }
}

fn settings(environment: &BTreeMap<String, String>) -> Settings {
    Settings {
        environment_key: non_empty(environment, "GOOGLE_API_KEY")
            .or_else(|| non_empty(environment, "GEMINI_API_KEY"))
            .map(SecretString::from),
        generic_api_key: None,
        api_base: non_empty(environment, "GEMINI_API_BASE")
            .map_or_else(|| DEFAULT_API_BASE.to_owned(), str::to_owned),
    }
}

fn non_empty<'env>(environment: &'env BTreeMap<String, String>, name: &str) -> Option<&'env str> {
    environment
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}
