use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use secrecy::SecretString;
use serde_json::Number;

use super::super::config::{
    Params, PreparedSearch, QuerySearchRequest, SearchConfig, SearchRequest, VectorStoreLogging,
};
use super::super::error::SearchError;
use super::{SearchCall, SearchDeps, Selected};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource};

pub(super) fn select<'deps, S>(
    deps: &SearchDeps<'deps, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
) -> Result<Selected<'deps, S::Client>, SearchError>
where
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
    config: &dyn SearchConfig,
    call: &SearchCall<'_>,
) -> Result<OpenAiHeaders, SearchError> {
    let mut headers = OpenAiHeaders::new();
    for (name, value) in call.extra_headers.into_iter().flatten() {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    config.validate_environment(headers, call.litellm_params)
}

// A query-embedding config is asked for its own transform with a read-only map whose timeout is the caller's; every other config gets the litellm params as they are
pub(super) async fn transform(
    config: &dyn SearchConfig,
    mode: ClientKind,
    call: &SearchCall<'_>,
    api_base: &str,
    logging: &mut VectorStoreLogging,
) -> Result<PreparedSearch, SearchError> {
    if let Some(query_config) = config.query_embedding_config() {
        let overridden = timeout_params(call.litellm_params, call.timeout);
        let request = QuerySearchRequest {
            search: search_request(call, api_base, &overridden, logging),
            embedding_executor: call.embedding_executor,
        };
        return match mode {
            ClientKind::Asynchronous => query_config.atransform_query_search_request(request).await,
            ClientKind::Sync => query_config.transform_query_search_request(request),
        };
    }
    let request = search_request(call, api_base, call.litellm_params, logging);
    match mode {
        ClientKind::Asynchronous => config.atransform_search_request(request).await,
        ClientKind::Sync => config.transform_search_request(request),
    }
}

fn search_request<'call>(
    call: &'call SearchCall<'_>,
    api_base: &'call str,
    litellm_params: &'call Params,
    logging: &'call mut VectorStoreLogging,
) -> SearchRequest<'call> {
    SearchRequest {
        vector_store_id: call.vector_store_id,
        query: call.query,
        optional_params: call.optional_params,
        api_base,
        logging,
        litellm_params,
        extra_body: call.extra_body,
    }
}

// dict(litellm_params, timeout=timeout) sets the key even when the caller passed no timeout
fn timeout_params(litellm_params: &Params, timeout: Option<Duration>) -> Params {
    let mut overridden = litellm_params.clone();
    let value = timeout.map_or(Json::Null, timeout_json);
    overridden.insert("timeout".to_owned(), value);
    overridden
}

// A timeout of whole seconds stays an integer, as a Python timeout passed as one does; a fraction is a float
fn timeout_json(duration: Duration) -> Json {
    if duration.subsec_nanos() == 0 {
        return Json::Number(Number::from(duration.as_secs()));
    }
    Json::Number(
        Number::from_f64(duration.as_secs_f64())
            .unwrap_or_else(|| Number::from(duration.as_secs())),
    )
}
