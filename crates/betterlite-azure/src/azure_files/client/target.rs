use std::time::Duration;

use betterlite_openai::files::DEFAULT_TIMEOUT;
use url::Url;

use crate::azure_files::error::AzureFilesError;
use crate::azure_urls::EndpointSelection;

// The SDK's AzureOpenAI builds <azure_endpoint>/openai, takes a base_url as given, and ends both in a slash
pub(super) fn azure_base_url(selection: &EndpointSelection) -> Result<Url, AzureFilesError> {
    match (&selection.azure_endpoint, &selection.base_url) {
        (Some(endpoint), _) => parse_base(&format!("{}/openai", endpoint.trim_end_matches('/'))),
        (None, Some(base)) => parse_base(base),
        (None, None) => Err(AzureFilesError::MissingBaseUrl),
    }
}

pub(super) fn parse_base(raw: &str) -> Result<Url, AzureFilesError> {
    let mut url = Url::parse(raw).map_err(|source| AzureFilesError::BaseUrl { source })?;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

pub(super) fn client_timeout(timeout: Option<f64>) -> Result<Duration, AzureFilesError> {
    match timeout {
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|_invalid| AzureFilesError::Timeout { seconds }),
        None => Ok(DEFAULT_TIMEOUT),
    }
}

// carried for the client projection only: betterlite's transport never retries, so a value the type
// cannot hold is reported as no retries rather than failing a request that could not use it anyway
pub(super) fn retries(max_retries: i64) -> u32 {
    u32::try_from(max_retries).unwrap_or(0)
}
