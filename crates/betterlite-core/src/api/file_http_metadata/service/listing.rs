use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::files::records::FileObject;
use http::Method;

use super::super::config::{FileParams, FileResponse, Headers, MetadataConfig, ReplyContext};
use super::super::error::MetadataError;
use super::Attempt;
use super::endpoints::validate_environment;
use super::wire::{file_response, request_url, send_once, wire_headers};
use crate::api::file_routing_ids::MAX_FILE_LIST_LIMIT;
use crate::api::vector_store_http_write::service::{LogContext, WriteRequest, WriteTransport};

pub(super) struct Listing<'deps, C: MetadataConfig, Client> {
    pub(super) config: &'deps C,
    pub(super) client: &'deps Client,
    pub(super) attempt: Attempt,
    pub(super) litellm_params: FileParams,
    pub(super) headers: Headers,
    pub(super) timeout: Option<Duration>,
}

// The eager listing: each page is transformed, capped, and only then asked for a continuation with freshly validated headers
pub(super) async fn list_pages<C, Client>(
    listing: &mut Listing<'_, C, Client>,
    log: &mut LogContext<'_, '_>,
    first: UpstreamResponse,
) -> Result<Vec<FileObject>, MetadataError>
where
    C: MetadataConfig,
    Client: WriteTransport,
{
    let mut reply = first;
    let mut listed: i64 = 0;
    let mut collected = Vec::new();
    loop {
        let response = file_response(&reply);
        let page = listing.config.transform_list_files_response(
            &response,
            ReplyContext {
                model_call_details: &mut log.state.model_call_details,
                litellm_params: &listing.litellm_params,
            },
        )?;
        let remaining = MAX_FILE_LIST_LIMIT.saturating_sub(listed).max(0);
        let take = usize::try_from(remaining).unwrap_or(0);
        collected.extend(page.iter().take(take).cloned());
        listed = i64::try_from(page.len())
            .unwrap_or(i64::MAX)
            .saturating_add(listed);
        let Some(next) = continuation(listing, &response, listed).await? else {
            break;
        };
        reply = next;
    }
    Ok(collected)
}

// The continuation is asked for only under the ceiling, with the headers validated again and the first send's timeout
async fn continuation<C, Client>(
    listing: &Listing<'_, C, Client>,
    response: &FileResponse,
    listed: i64,
) -> Result<Option<UpstreamResponse>, MetadataError>
where
    C: MetadataConfig,
    Client: WriteTransport,
{
    let optional_params = FileParams::new();
    let next = if listed < MAX_FILE_LIST_LIMIT {
        listing.config.transform_list_files_next_request(
            response,
            &optional_params,
            &listing.litellm_params,
        )
    } else {
        None
    };
    let Some((url, params)) = next else {
        return Ok(None);
    };
    let headers = validate_environment(
        listing.config,
        listing.headers.clone(),
        &optional_params,
        &listing.litellm_params,
    )?;
    let request = WriteRequest {
        method: Method::GET,
        url: request_url(&url, &params.into_iter().collect::<Vec<_>>())?,
        headers: wire_headers(&headers)?,
        body: RequestBody::Empty,
        timeout: listing.timeout,
    };
    let reply = send_once(listing.config, listing.client, request, listing.attempt).await?;
    Ok(Some(reply))
}
