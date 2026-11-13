use super::environment::WatsonxEnvironment;
use super::error::WatsonxError;
use super::types::DEFAULT_API_VERSION;

const URL_NOT_SET: &str = "Error: Watsonx URL not set. Set WATSONX_API_BASE in environment variables or pass in as parameter - 'api_base='.";

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
