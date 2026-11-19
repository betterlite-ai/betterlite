use std::time::Duration;

use betterlite_http::url_path::encode_segment;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use betterlite_openai::vector_stores::store::create_request;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;
use url::Url;
use url::form_urlencoded;

use super::super::config::CreateReadConfig;
use super::{
    ClientKind, ClientSource, CreateCall, CreateReadDeps, CreateReadError, CreateReadTarget,
    ListCall, ListQuery, Operation, RetrieveCall, Selected,
};
use crate::api::vector_store_http_write::config::WriteConfig;
use crate::api::vector_store_http_write::error::WriteError;

pub(super) fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow CreateReadDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, CreateReadError>
where
    'deps: 'borrow,
    C: CreateReadConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
}

// B527's validate_environment rewrites Authorization and Content-Type in the map it is handed, so the caller's entries go in first
pub(super) fn validated_headers(
    provider: &dyn WriteConfig,
    target: &CreateReadTarget<'_>,
) -> OpenAiHeaders {
    let mut caller_headers = OpenAiHeaders::new();
    for (name, value) in target.extra_headers.into_iter().flatten() {
        caller_headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    provider.validate_environment(caller_headers, target.params)
}

pub(super) fn create_endpoint(
    provider: &dyn WriteConfig,
    call: &CreateCall<'_>,
) -> Result<(String, Url, IndexMap<String, Json>), CreateReadError> {
    let api_base = provider
        .complete_url(call.target.params)
        .map_err(WriteError::Url)?;
    let request = create_request(call.optional_params, &api_base);
    let body: IndexMap<String, Json> =
        serde_json::from_value(request.body).map_err(WriteError::Encode)?;
    let url = Url::parse(&request.url).map_err(|source| WriteError::RequestUrl {
        url: request.url,
        source,
    })?;
    Ok((api_base, url, body))
}

pub(super) fn retrieve_endpoint(
    provider: &dyn WriteConfig,
    call: &RetrieveCall<'_>,
) -> Result<(String, Url), CreateReadError> {
    let api_base = provider
        .complete_url(call.target.params)
        .map_err(WriteError::Url)?;
    let encoded =
        encode_segment(call.vector_store_id, "vector_store_id").map_err(WriteError::Segment)?;
    // the literal slash plus one encoded id lands after any query string, exactly as LiteLLM's f-string does
    let text = format!("{api_base}/{encoded}");
    let url = Url::parse(&text).map_err(|source| WriteError::RequestUrl { url: text, source })?;
    Ok((api_base, url))
}

pub(super) fn list_endpoint(
    provider: &dyn WriteConfig,
    call: &ListCall<'_>,
) -> Result<(String, Url), CreateReadError> {
    let api_base = provider
        .complete_url(call.target.params)
        .map_err(WriteError::Url)?;
    let mut url = Url::parse(&api_base).map_err(|source| WriteError::RequestUrl {
        url: api_base.clone(),
        source,
    })?;
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    for (name, param) in entries(&call.query) {
        // httpx merges the call's params into the URL's own query, so a name both carry keeps the URL's value
        if pairs.iter().any(|(existing, _)| existing.as_str() == name) {
            continue;
        }
        pairs.push((name.to_owned(), param.text()));
    }
    if !pairs.is_empty() {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        serializer.extend_pairs(
            pairs
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        );
        url.set_query(Some(&serializer.finish()));
    }
    Ok((api_base, url))
}

// The list handler logs a dict of its four arguments: an int stays a number here and a string on the wire
pub(super) fn list_params(query: &ListQuery<'_>) -> IndexMap<String, Json> {
    entries(query)
        .into_iter()
        .map(|(name, param)| (name.to_owned(), param.json()))
        .collect()
}

enum Param<'query> {
    Text(&'query str),
    Count(i64),
}

impl Param<'_> {
    fn text(&self) -> String {
        match self {
            Self::Text(text) => (*text).to_owned(),
            Self::Count(count) => count.to_string(),
        }
    }

    fn json(&self) -> Json {
        match self {
            Self::Text(text) => Json::Text((*text).to_owned()),
            Self::Count(count) => Json::Number(Number::from(*count)),
        }
    }
}

// Every field LiteLLM tests with is not None, so an empty after and a zero limit are kept; an absent one is left out
fn entries<'query>(query: &ListQuery<'query>) -> Vec<(&'static str, Param<'query>)> {
    let mut entries = Vec::new();
    if let Some(after) = query.after {
        entries.push(("after", Param::Text(after)));
    }
    if let Some(before) = query.before {
        entries.push(("before", Param::Text(before)));
    }
    if let Some(limit) = query.limit {
        entries.push(("limit", Param::Count(limit)));
    }
    if let Some(order) = query.order {
        entries.push(("order", Param::Text(order)));
    }
    entries
}

// Only the async create handler hands the caller's timeout to its client; every read and the sync create leave the selected client's own deadline
pub(super) const fn call_timeout(
    mode: ClientKind,
    operation: Operation,
    timeout: Option<Duration>,
) -> Option<Duration> {
    match (mode, operation) {
        (ClientKind::Asynchronous, Operation::Create) => timeout,
        (ClientKind::Asynchronous, Operation::Retrieve | Operation::List)
        | (ClientKind::Sync, Operation::Create | Operation::Retrieve | Operation::List) => None,
    }
}
