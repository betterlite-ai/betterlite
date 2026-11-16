use betterlite_http::url_path::encode_segments;

use super::environment::WatsonxEnvironment;
use super::error::WatsonxError;
use super::types::DEFAULT_API_VERSION;

const URL_NOT_SET: &str = "Error: Watsonx URL not set. Set WATSONX_API_BASE in environment variables or pass in as parameter - 'api_base='.";

// LiteLLM joins model.split("/")[1:] raw into the deployment path, so a ".." or "?" there moves the call
pub fn deployment_id(model: &str) -> Result<Option<String>, WatsonxError> {
    let Some(rest) = model.strip_prefix("deployment/") else {
        return Ok(None);
    };
    encode_segments(rest, "deployment id")
        .map(Some)
        .map_err(|source| WatsonxError::Api {
            status_code: 400,
            message: source.to_string(),
            headers: None,
        })
}

pub fn get_base_url(
    api_base: Option<&str>,
    environment: &WatsonxEnvironment,
) -> Result<String, WatsonxError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| environment.api_base().map(str::to_owned));
    base.ok_or_else(|| WatsonxError::Api {
        status_code: 401,
        message: URL_NOT_SET.to_owned(),
        headers: None,
    })
}

#[must_use]
pub fn add_api_version_to_url(url: &str, api_version: Option<&str>) -> String {
    format!(
        "{url}?version={}",
        api_version
            .filter(|version| !version.is_empty())
            .unwrap_or(DEFAULT_API_VERSION)
    )
}
