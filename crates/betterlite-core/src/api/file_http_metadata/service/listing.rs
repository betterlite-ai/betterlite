use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::files::records::FileObject;
use http::Method;

use super::super::config::{
    EnvironmentRequest, FileLogging, FileParams, FileResponse, Headers, MetadataConfig,
    NextPageRequest,
};
use super::super::error::MetadataError;
use super::wire::{file_response, request_url, send_once, wire_headers};
use super::{Attempt, NO_MESSAGES, Operation};
use crate::api::file_routing_ids::MAX_FILE_LIST_LIMIT;
use crate::api::vector_store_http_write::service::{
    ClientKind, LogContext, WriteRequest, WriteTransport,
};

pub(super) struct Listing<'deps, C: MetadataConfig, Client> {
    pub(super) config: &'deps C,
    pub(super) client: &'deps Client,
    pub(super) mode: ClientKind,
    pub(super) litellm_params: FileParams,
    pub(super) headers: Headers,
    pub(super) timeout: Option<Duration>,
}

// The eager listing: each page is transformed, capped, and only then asked for a continuation with freshly validated headers
pub(super) async fn list_pages<C, Client>(
    listing: &mut Listing<'_, C, Client>,
    log: &mut LogContext<'_, '_>,
    first: UpstreamResponse,
    first_url: String,
) -> Result<Vec<FileObject>, MetadataError>
where
    C: MetadataConfig,
    Client: WriteTransport,
{
    let mut reply = first;
    let mut url = first_url;
    let mut listed: i64 = 0;
    let mut collected = Vec::new();
    loop {
        let response = file_response(&reply, &url);
        let page = listing.config.transform_list_files_response(
            &response,
            &mut FileLogging {
                model_call_details: &mut log.state.model_call_details,
            },
            &listing.litellm_params,
        )?;
        let remaining = MAX_FILE_LIST_LIMIT.saturating_sub(listed).max(0);
        let take = usize::try_from(remaining).map_or(0, |count| count);
        collected.extend(page.iter().take(take).cloned());
        listed = i64::try_from(page.len()).map_or(i64::MAX, |count| listed.saturating_add(count));
        let optional_params = FileParams::new();
        let Some((next_url, next_params)) = next_listing_request(
            listing.config,
            &response,
            &optional_params,
            &listing.litellm_params,
            listed,
        ) else {
            break;
        };
        let next_headers = next_listing_page_headers(
            listing.config,
            &listing.headers,
            &optional_params,
            &listing.litellm_params,
        )?;
        let request =
            continuation_request(&next_url, &next_params, &next_headers, listing.timeout)?;
        url = request.url.to_string();
        reply = send_once(
            listing.config,
            listing.client,
            request,
            Attempt {
                mode: listing.mode,
                operation: Operation::List,
            },
        )
        .await?;
    }
    Ok(collected)
}

fn next_listing_request<C>(
    config: &C,
    response: &FileResponse,
    optional_params: &FileParams,
    litellm_params: &FileParams,
    listed: i64,
) -> Option<NextPageRequest>
where
    C: MetadataConfig,
{
    if listed >= MAX_FILE_LIST_LIMIT {
        return None;
    }
    config.transform_list_files_next_request(response, optional_params, litellm_params)
}

fn next_listing_page_headers<C>(
    config: &C,
    headers: &Headers,
    optional_params: &FileParams,
    litellm_params: &FileParams,
) -> Result<Headers, MetadataError>
where
    C: MetadataConfig,
{
    let api_key = litellm_params
        .get("api_key")
        .and_then(|value| value.as_str());
    config
        .validate_environment(EnvironmentRequest {
            api_key,
            headers: headers.clone(),
            model: "",
            messages: &NO_MESSAGES,
            optional_params,
            litellm_params,
        })
        .map_err(MetadataError::Provider)
}

fn continuation_request(
    url: &str,
    params: &BTreeMap<String, String>,
    headers: &Headers,
    timeout: Option<Duration>,
) -> Result<WriteRequest, MetadataError> {
    let pairs: Vec<(String, String)> = params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Ok(WriteRequest {
        method: Method::GET,
        url: request_url(url, &pairs)?,
        headers: wire_headers(headers)?,
        body: RequestBody::Empty,
        timeout,
    })
}
