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
use betterlite_vertex::studio_file_search::request::{SearchInput, SearchQuery, search_request};
use betterlite_vertex::studio_identity::{API_VERSION, Headers};
use jiff::tz::TimeZone;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    params: Option<BTreeMap<String, String>>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    vector_store_id: Option<String>,
    query: Option<QueryInput>,
    filters: Option<Json>,
    max_num_results: Option<i64>,
    extra_body: Option<Json>,
    api_key: Option<String>,
    model: Option<String>,
    api_base: Option<String>,
    name: Option<String>,
    status: Option<u16>,
    body: Option<String>,
    timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Capabilities,
    Environment,
    Url,
    Search,
    SearchReply,
    Create,
    CreateReply,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QueryInput {
    Text(String),
    Sequence(Vec<String>),
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
    match call.function {
        Function::Capabilities => capabilities_view(),
        Function::Environment => environment_view(call),
        Function::Url => {
            let settings = settings(&call.environment);
            let base = call.api_base.as_deref().unwrap_or(&settings.api_base);
            Ok(json!({ "url": format!("{base}/{API_VERSION}") }))
        }
        Function::Search => search_view(call),
        Function::SearchReply => {
            let query = match call.query.as_ref() {
                Some(QueryInput::Text(text)) => text.as_str(),
                Some(QueryInput::Sequence(_)) | None => "",
            };
            decode_reply(call, Shape::Search, query, TimeZone::UTC)
        }
        Function::Create => create_view(call),
        Function::CreateReply => {
            let zone =
                TimeZone::get(call.timezone.as_deref().unwrap_or_default()).or_value_error()?;
            decode_reply(call, Shape::Create, "", zone)
        }
    }
}

fn capabilities_view() -> HelperResult {
    let found = capabilities();
    Ok(json!({
        "endpoints": found.endpoints,
        "credentials": found.credentials,
        "supported": found.supported_params,
    }))
}

fn environment_view(call: &Call) -> HelperResult {
    let settings = settings(&call.environment);
    let params = call.params.as_ref().map(|given| RequestParams {
        api_key: given
            .get("api_key")
            .map(String::as_str)
            .map(SecretString::from),
        model: None,
    });
    let headers = validate_environment(&call.headers, params.as_ref(), &settings);
    Ok(json!({ "headers": exposed(&headers) }))
}

fn search_view(call: &Call) -> HelperResult {
    let settings = settings(&call.environment);
    let params = RequestParams {
        api_key: call.api_key.as_deref().map(SecretString::from),
        model: call.model.clone(),
    };
    let query = match call.query.as_ref() {
        Some(QueryInput::Text(text)) => SearchQuery::Text(text.clone()),
        Some(QueryInput::Sequence(items)) => SearchQuery::Sequence(items.clone()),
        None => SearchQuery::Text(String::new()),
    };
    let plan = search_request(&SearchInput {
        vector_store_id: call.vector_store_id.as_deref().unwrap_or_default(),
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

fn create_view(call: &Call) -> HelperResult {
    let settings = settings(&call.environment);
    let base = call.api_base.as_deref().unwrap_or(&settings.api_base);
    let plan = create_request(call.name.as_deref(), &format!("{base}/{API_VERSION}"));
    Ok(json!({ "url": plan.url, "body": plan.body }))
}

fn decode_reply(call: &Call, shape: Shape, query: &str, zone: TimeZone) -> HelperResult {
    let response = RawResponse {
        status: call.status.unwrap_or_default(),
        headers: &call.headers,
        body: call.body.as_deref().unwrap_or_default().as_bytes(),
    };
    let local = LocalTimeContext { zone };
    match decode_studio_file_search(shape, &response, query, &local).map_err(failure)? {
        Decoded::Search(record) => Ok(serde_json::to_value(&*record)?),
        Decoded::Create(record) => Ok(serde_json::to_value(&*record)?),
    }
}

fn failure(error: StudioFileSearchError) -> Box<HelperError> {
    let (class, status) = match &error {
        StudioFileSearchError::SearchReply { status, .. }
        | StudioFileSearchError::CreateReply { status, .. } => ("GeminiError", Some(*status)),
        StudioFileSearchError::MissingKey | StudioFileSearchError::Json { .. } => {
            ("ValueError", None)
        }
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn settings(environment: &BTreeMap<String, String>) -> Settings {
    Settings {
        environment_key: ["GOOGLE_API_KEY", "GEMINI_API_KEY"]
            .into_iter()
            .find_map(|name| environment.get(name).filter(|value| !value.is_empty()))
            .map(|value| SecretString::from(value.as_str())),
        generic_api_key: None,
        api_base: environment
            .get("GEMINI_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    }
}

fn exposed(headers: &Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}
