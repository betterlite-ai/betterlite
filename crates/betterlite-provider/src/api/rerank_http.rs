use std::collections::BTreeMap;
use std::error::Error;
use std::time::{Duration, Instant};

use betterlite_core::api::file_http_content::service::ClientSource;
use betterlite_core::api::vector_store_http_write::error::handle_error;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use self::client::{
    ClientKind, RerankHttpRequest, RerankTransport, SuppliedClient, client_kind, select_client,
};
use self::convert::{Sent, provider_response, send_facts, status_facts};
use self::error::RerankHttpError;
use self::logging::log_pre_call;
use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, Params, RerankLogging, ResponseTransform,
};

pub mod client;
mod convert;
pub mod error;
mod logging;
mod tests;

const EMPTY_HEADERS: Headers = BTreeMap::new();
const EMPTY_PARAMS: Params = BTreeMap::new();

pub struct RerankCall<'call> {
    pub model: &'call str,
    pub optional_params: &'call Params,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub litellm_params: Option<&'call Params>,
    pub headers: Option<&'call Headers>,
    pub timeout: Option<Duration>,
    pub is_async: Json,
    pub model_response: RerankResponse,
    pub logging: &'call mut RerankLogging,
}

pub struct RerankLogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct RerankDeps<'deps, C: BaseRerankConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

struct Prepared {
    request: RerankHttpRequest,
    data: Params,
}

// rerank and arerank share one front half: validate, complete URL, transform and pre-call all run before the transport
fn prepare<C>(
    config: &C,
    call: &RerankCall<'_>,
    log: &mut RerankLogContext<'_, '_>,
) -> Result<Prepared, RerankHttpError<C::Error>>
where
    C: BaseRerankConfig,
{
    let headers = config
        .validate_environment(EnvironmentRequest {
            headers: call.headers.unwrap_or(&EMPTY_HEADERS),
            model: call.model,
            api_key: call.api_key,
            optional_params: Some(call.optional_params),
            litellm_params: call.litellm_params,
        })
        .map_err(RerankHttpError::Config)?;
    let api_base = config.get_complete_url(call.api_base, call.model, Some(call.optional_params));
    let url = Url::parse(&api_base).map_err(|source| RerankHttpError::RequestUrl {
        url: api_base.clone(),
        source,
    })?;
    let data = config
        .transform_rerank_request(
            call.model,
            call.optional_params,
            &headers,
            call.litellm_params,
        )
        .map_err(RerankHttpError::Config)?;
    log_pre_call(&api_base, &headers, &data, call.optional_params, log)?;
    Ok(Prepared {
        request: RerankHttpRequest {
            url,
            headers: wire_headers(&headers)?,
            body: request_body(&data),
            timeout: call.timeout,
        },
        data,
    })
}

fn wire_headers<Failure>(headers: &Headers) -> Result<HeaderMap, RerankHttpError<Failure>>
where
    Failure: Error + Send + Sync + 'static,
{
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            RerankHttpError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| RerankHttpError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// data=json.dumps(data): an httpx str body carries the Python separators and ASCII escapes, and no content type
fn request_body(data: &Params) -> Vec<u8> {
    let object: IndexMap<String, Json> = data
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    python_json::dumps(&Json::Object(object)).into_bytes()
}

fn transform_response<C>(
    config: &C,
    call: RerankCall<'_>,
    data: &Params,
    reply: &UpstreamResponse,
) -> Result<RerankResponse, RerankHttpError<C::Error>>
where
    C: BaseRerankConfig,
{
    let raw_response = provider_response(reply);
    config
        .transform_rerank_response(
            call.model_response,
            ResponseTransform {
                model: call.model,
                raw_response: &raw_response,
                logging: call.logging,
                api_key: call.api_key,
                request_data: data,
                optional_params: &EMPTY_PARAMS,
                litellm_params: &EMPTY_PARAMS,
            },
        )
        .map_err(RerankHttpError::Config)
}

pub async fn rerank<C, S>(
    call: RerankCall<'_>,
    deps: RerankDeps<'_, C, S>,
    log: &mut RerankLogContext<'_, '_>,
) -> Result<RerankResponse, RerankHttpError<C::Error>>
where
    C: BaseRerankConfig,
    S: ClientSource<Client: RerankTransport>,
{
    let prepared = prepare(deps.config, &call, log)?;
    let kind = client_kind(&call.is_async);
    let selected = select_client(deps.supplied.as_ref(), deps.source, kind)?;
    let logging = matches!(kind, ClientKind::Asynchronous).then_some(&*call.logging);
    let started = Instant::now();
    let answer = selected.get().send(prepared.request, logging).await;
    let sent = Sent {
        kind,
        timeout: call.timeout,
        elapsed: started.elapsed(),
    };
    let reply = answer.map_err(|error| {
        let failure = handle_error(&send_facts(&error, &sent), None);
        RerankHttpError::Provider(deps.config.get_error_class(
            &failure.message,
            failure.status_code,
            failure.headers,
        ))
    })?;
    if reply.status >= 400 {
        let failure = handle_error(&status_facts(&reply), None);
        return Err(RerankHttpError::Provider(deps.config.get_error_class(
            &failure.message,
            failure.status_code,
            failure.headers,
        )));
    }
    transform_response(deps.config, call, &prepared.data, &reply)
}
