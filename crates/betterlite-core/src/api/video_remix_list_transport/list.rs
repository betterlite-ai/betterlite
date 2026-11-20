use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use tokio::runtime::{Builder, Handle};

use crate::api::vector_store_http_write::error::display_facts;
use crate::api::vector_store_http_write::service::ClientKind;

use super::client::{VideoAsyncClient, VideoClientSource, VideoRequest};
use super::config::VideoRemixListConfig;
use super::convert;
use super::error::{Operation, VideoRemixListTransportError};
use super::fields;
use super::logging::{ListPreCall, PreCallSource, record_pre_call};
use super::records::{Headers, Params, VideoEnvironment, VideoListQuery, VideoRequestContext};
use super::wire;
use super::{ListCall, LogContext, VideoRemixListTransport};

pub(super) struct PreparedList {
    url: String,
    headers: Headers,
    params: Params,
}

// validate_environment gets the extra headers and only the api_key argument; the caller's headers are re-applied over its answer
fn list_headers<Source>(config: &dyn VideoRemixListConfig, call: &ListCall<'_, Source>) -> Headers
where
    Source: VideoClientSource,
{
    let empty = Headers::new();
    let extra = call.extra_headers.unwrap_or(&empty);
    let mut headers = config.validate_environment(VideoEnvironment {
        headers: extra,
        model: "",
        api_key: call.api_key,
        litellm_params: Some(call.litellm_params),
    });
    if let Some(extra_headers) = call.extra_headers {
        for (name, value) in extra_headers {
            headers.insert(name.clone(), value.clone());
        }
    }
    headers
}

// The provider's query transform turns the base URL and the validated headers into the wire URL and query
fn list_request<Source>(
    config: &dyn VideoRemixListConfig,
    call: &ListCall<'_, Source>,
    base_url: &str,
    headers: &Headers,
) -> Result<(String, Params), VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
    config
        .transform_video_list_request(
            VideoListQuery {
                after: call.after,
                limit: call.limit,
                order: call.order,
                extra_query: call.extra_query,
            },
            VideoRequestContext {
                api_base: base_url,
                litellm_params: call.litellm_params,
                headers,
            },
        )
        .map_err(|source| VideoRemixListTransportError::Prepare {
            operation: Operation::List,
            source,
        })
}

// The try block's GET: the transform's params as the query and no timeout, whatever the caller passed
fn list_wire_request(
    config: &dyn VideoRemixListConfig,
    prepared: &PreparedList,
) -> Result<VideoRequest, VideoRemixListTransportError> {
    wire::query_get(&prepared.url, &prepared.headers, &prepared.params)
        .map_err(|error| convert::failure(config, &display_facts(error.to_string())))
}

// The try block's send on the async path: one GET with no timeout, and the caller's timeout only in a timeout message
pub(super) async fn send_list<Client>(
    config: &dyn VideoRemixListConfig,
    client: &Client,
    prepared: &PreparedList,
    timeout: Option<Duration>,
) -> Result<UpstreamResponse, VideoRemixListTransportError>
where
    Client: VideoAsyncClient,
{
    let request = list_wire_request(config, prepared)?;
    let started = Instant::now();
    let sent = client.send(request).await;
    let facts = convert::SendFacts {
        operation: Operation::List,
        mode: ClientKind::Asynchronous,
        timeout,
        elapsed: started.elapsed(),
    };
    sent.map_err(|error| convert::failure(config, &convert::transport_facts(&facts, &error)))
}

// Everything before the try block: the environment (api_key only), the base URL, the query transform and the pre-call
pub(super) fn prepare_list<Source>(
    config: &dyn VideoRemixListConfig,
    call: &ListCall<'_, Source>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedList, VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
    let headers = list_headers(config, call);
    let api_base = fields::read::<String>(call.litellm_params, "api_base")?;
    let base_url = config
        .get_complete_url("", api_base.as_deref(), call.litellm_params)
        .map_err(|source| VideoRemixListTransportError::Prepare {
            operation: Operation::List,
            source,
        })?;
    let (url, params) = list_request(config, call, &base_url, &headers)?;
    record_pre_call(
        log,
        PreCallSource::List(&ListPreCall {
            url: &url,
            headers: &headers,
            params: &params,
        }),
    )?;
    Ok(PreparedList {
        url,
        headers,
        params,
    })
}

// The sync entry point's run to completion: a private current-thread runtime drives the async handler
pub(super) fn run_list_now<Source>(
    transport: &VideoRemixListTransport<Source>,
    config: &dyn VideoRemixListConfig,
    call: ListCall<'_, Source>,
    log: LogContext<'_, '_>,
) -> Result<Params, VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
    if Handle::try_current().is_ok() {
        return Err(VideoRemixListTransportError::RunningRuntime { status_code: 500 });
    }
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| VideoRemixListTransportError::Runtime { source })?;
    let async_call = ListCall {
        api_key: None,
        ..call
    };
    runtime.block_on(transport.async_list(config, async_call, log))
}
