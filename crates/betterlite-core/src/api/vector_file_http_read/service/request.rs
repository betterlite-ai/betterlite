use std::collections::BTreeMap;

use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use secrecy::SecretString;

use crate::api::vector_store_http_write::service::ClientKind;

use super::super::config::{ReadConfig, ReadParams};
use super::super::error::ReadError;
use super::{ClientSource, ReadDeps, Selected};

// A supplied client is reused only by its own kind; every other call takes one from the pool
pub(super) fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow ReadDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, ReadError>
where
    'deps: 'borrow,
    C: ReadConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
}

// LiteLLM hands validate_environment the caller's headers and then re-applies the same dict it mutated, so the validated map is the answer
pub(super) fn validated_headers<C>(
    config: &C,
    extra_headers: Option<&BTreeMap<String, String>>,
    params: &ReadParams,
) -> OpenAiHeaders
where
    C: ReadConfig,
{
    let mut caller_headers = OpenAiHeaders::new();
    for (name, value) in extra_headers.into_iter().flatten() {
        caller_headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    config.validate_environment(caller_headers, params)
}
