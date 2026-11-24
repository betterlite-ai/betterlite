use std::io;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;
use http::HeaderMap;
use tokio::fs::read_to_string;

use crate::openapi_tools::document::{OpenApiDocument, parse_openapi_document};
use crate::openapi_tools::error::OpenApiError;

const NOT_FOUND: io::ErrorKind = io::ErrorKind::NotFound;

// LiteLLM's load_openapi_spec_async: an http(s) location through the shared fetcher, anything else a JSON file
pub async fn load_openapi_spec_async<R>(
    fetcher: &UserUrlFetcher<R>,
    client: &ProxiedClient,
    location: &str,
    max_bytes: Option<usize>,
) -> Result<OpenApiDocument, OpenApiError>
where
    R: Resolve,
{
    if location.starts_with("http://") || location.starts_with("https://") {
        let body = fetch_spec(fetcher, client, location, max_bytes).await?;
        return parse_openapi_document(&body);
    }
    let contents = read_local_spec(location).await?;
    parse_openapi_document(contents.as_bytes())
}

async fn fetch_spec<R>(
    fetcher: &UserUrlFetcher<R>,
    client: &ProxiedClient,
    location: &str,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, OpenApiError>
where
    R: Resolve,
{
    let response = fetcher
        .fetch(client, location, &HeaderMap::new())
        .await
        .map_err(|source| OpenApiError::SpecFetch {
            location: location.to_owned(),
            source,
        })?;
    let limit = max_bytes.unwrap_or(client.limits.body_bytes);
    let buffered = response
        .buffered(limit)
        .await
        .map_err(|source| OpenApiError::SpecBody {
            location: location.to_owned(),
            source,
        })?;
    if buffered.status >= 400 {
        return Err(OpenApiError::SpecStatus {
            location: location.to_owned(),
            status: buffered.status,
        });
    }
    Ok(buffered.body)
}

async fn read_local_spec(location: &str) -> Result<String, OpenApiError> {
    match read_to_string(location).await {
        Ok(contents) => Ok(contents),
        Err(source) if source.kind() == NOT_FOUND => Err(OpenApiError::SpecNotFound {
            location: location.to_owned(),
        }),
        Err(source) => Err(OpenApiError::SpecRead {
            location: location.to_owned(),
            source,
        }),
    }
}
