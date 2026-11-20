use betterlite_http::proxy::outbound::RequestBody;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::super::config::{FileWriteConfig, QueryParams};
use super::super::error::FileWriteError;
use super::super::params::merged_request;
use super::{
    ClientKind, ClientSource, DeleteCall, LogContext, Selected, UpdateCall, WriteDeps, WriteTarget,
};

// The request an operation puts on the wire, after the pre-call logged the same body or query
pub(super) struct Parts {
    pub(super) url: Url,
    pub(super) query: Vec<(String, String)>,
    pub(super) body: RequestBody,
}

pub(super) fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow WriteDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, FileWriteError>
where
    'deps: 'borrow,
    C: FileWriteConfig,
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
pub(super) fn validated_headers<C>(config: &C, target: &WriteTarget<'_>) -> OpenAiHeaders
where
    C: FileWriteConfig,
{
    let mut caller_headers = OpenAiHeaders::new();
    for (name, value) in target.extra_headers.into_iter().flatten() {
        caller_headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    config.validate_environment(caller_headers, target.params)
}

// LiteLLM's pre_call(input="", api_key="") over the collection base, the validated headers and the transformed body or params
pub(super) fn log_pre_call(
    api_base: &str,
    headers: &OpenAiHeaders,
    complete: Option<&IndexMap<String, Json>>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), FileWriteError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.expose_secret().to_owned()));
    }
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: complete.cloned().map(RawBody::Object),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}

// The update operation's transform and pre-call, answering the parts its request goes out with
pub(super) fn update_parts<C>(
    config: &C,
    update: &UpdateCall<'_>,
    api_base: &str,
    headers: &OpenAiHeaders,
    log: &mut LogContext<'_, '_>,
) -> Result<Parts, FileWriteError>
where
    C: FileWriteConfig,
{
    let merged = merged_request(update.update_request, update.extra_body)?;
    let (target_url, payload) =
        config.transform_update_request(&merged, api_base, update.target.file_id)?;
    log_pre_call(api_base, headers, Some(&payload), log)?;
    Ok(Parts {
        url: request_url(&target_url)?,
        query: Vec::new(),
        body: wire_body(&payload)?,
    })
}

// The delete operation's transform and pre-call: its params go out as the query string, and its body is empty
pub(super) fn delete_parts<C>(
    config: &C,
    delete: &DeleteCall<'_>,
    api_base: &str,
    headers: &OpenAiHeaders,
    log: &mut LogContext<'_, '_>,
) -> Result<Parts, FileWriteError>
where
    C: FileWriteConfig,
{
    let (target_url, query) = config.transform_delete_request(api_base, delete.target.file_id)?;
    log_pre_call(api_base, headers, Some(&logged_query(&query)), log)?;
    Ok(Parts {
        url: request_url(&target_url)?,
        query: query.into_iter().collect(),
        body: RequestBody::Empty,
    })
}

// The delete transform's params dict is what pre_call logs before httpx appends it to the URL
pub(super) fn logged_query(query: &QueryParams) -> IndexMap<String, Json> {
    query
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

pub(super) fn request_url(text: &str) -> Result<Url, FileWriteError> {
    Url::parse(text).map_err(|source| FileWriteError::RequestUrl {
        url: text.to_owned(),
        source,
    })
}

pub(super) fn wire_headers(headers: &OpenAiHeaders) -> Result<HttpHeaders, FileWriteError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            FileWriteError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value = HeaderValue::from_str(value.expose_secret()).map_err(|source| {
            FileWriteError::HeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

pub(super) fn wire_body(body: &IndexMap<String, Json>) -> Result<RequestBody, FileWriteError> {
    serde_json::to_vec(body)
        .map(RequestBody::Json)
        .map_err(FileWriteError::Encode)
}
