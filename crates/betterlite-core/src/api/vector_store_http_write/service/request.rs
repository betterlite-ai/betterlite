use std::time::Duration;

use betterlite_http::url_path::encode_segment;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use super::super::config::WriteConfig;
use super::super::error::WriteError;
use super::{ClientKind, ClientSource, Selected, WriteCall, WriteDeps, WriteTarget};

pub(super) fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow WriteDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, WriteError>
where
    'deps: 'borrow,
    C: WriteConfig,
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
    C: WriteConfig,
{
    let mut caller_headers = OpenAiHeaders::new();
    for (name, value) in target.extra_headers.into_iter().flatten() {
        caller_headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    config.validate_environment(caller_headers, target.params)
}

pub(super) fn write_endpoint<C>(
    config: &C,
    target: &WriteTarget<'_>,
) -> Result<(String, Url), WriteError>
where
    C: WriteConfig,
{
    let api_base = config
        .complete_url(target.params)
        .map_err(WriteError::Url)?;
    let encoded =
        encode_segment(target.vector_store_id, "vector_store_id").map_err(WriteError::Segment)?;
    // The literal slash plus one encoded id lands after any query string, exactly as LiteLLM's f-string does
    let text = format!("{api_base}/{encoded}");
    let url = Url::parse(&text).map_err(|source| WriteError::RequestUrl { url: text, source })?;
    Ok((api_base, url))
}

// The delete handlers take extra_body and never read it, so only an update builds one
pub(super) fn update_body(
    call: &WriteCall<'_>,
) -> Result<Option<IndexMap<String, Json>>, WriteError> {
    match call {
        WriteCall::Update(update) => update
            .optional_params
            .request_body(update.extra_body)
            .map(Some),
        WriteCall::Delete(_) => Ok(None),
    }
}

// LiteLLM's sync handlers pass no timeout, leaving the selected client's own deadline in effect
pub(super) const fn call_timeout(mode: ClientKind, timeout: Option<Duration>) -> Option<Duration> {
    match mode {
        ClientKind::Asynchronous => timeout,
        ClientKind::Sync => None,
    }
}
