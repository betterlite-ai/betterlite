use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;

use crate::endpoint_target::target::EndpointTarget;

use super::error::CacheLookupError;
use super::records::{CACHE_PAGE_LIMIT, decode_cached_page};
use super::transport::{CacheTransport, cache_request};

pub const MAX_PAGINATION_PAGES: usize = 100;

const FORBIDDEN_STATUS: u16 = 403;

enum PageStep {
    Found(Option<String>),
    Next(String),
    Stop,
}

// LiteLLM's sync and async list loops differ only in the client call, so one async loop serves both
pub async fn check_cache<T>(
    transport: &T,
    target: &EndpointTarget,
    cache_key: &str,
    extra_headers: &BTreeMap<String, String>,
    deadline: Duration,
) -> Result<Option<String>, CacheLookupError>
where
    T: CacheTransport,
{
    let mut page_token: Option<String> = None;
    for _page in 0..MAX_PAGINATION_PAGES {
        let request = cache_request(target, page_token.as_deref(), extra_headers, deadline)?;
        let reply = transport
            .send(request)
            .await
            .map_err(|source| CacheLookupError::Transport { source })?;
        match page_step(&reply, cache_key)? {
            PageStep::Found(name) => return Ok(name),
            PageStep::Next(next) => page_token = Some(next),
            PageStep::Stop => return Ok(None),
        }
    }
    Ok(None)
}

fn page_step(reply: &UpstreamResponse, cache_key: &str) -> Result<PageStep, CacheLookupError> {
    if reply.status == FORBIDDEN_STATUS {
        return Ok(PageStep::Stop);
    }
    if reply.body.len() > CACHE_PAGE_LIMIT {
        return Err(CacheLookupError::PageTooLarge {
            limit: CACHE_PAGE_LIMIT,
        });
    }
    if !(200..300).contains(&reply.status) {
        return Err(CacheLookupError::Http {
            status: reply.status,
            message: str::from_utf8(&reply.body).map_or_else(
                |_| format!("{} bytes that are not UTF-8", reply.body.len()),
                str::to_owned,
            ),
        });
    }
    let page = decode_cached_page(&reply.body)?;
    let Some(items) = page.cached_contents else {
        return Ok(PageStep::Stop);
    };
    for item in items {
        if item.display_name.as_deref() == Some(cache_key) {
            return Ok(PageStep::Found(item.name));
        }
    }
    let Some(next) = page.next_page_token.filter(|token| !token.is_empty()) else {
        return Ok(PageStep::Stop);
    };
    Ok(PageStep::Next(next))
}
