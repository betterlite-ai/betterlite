use std::collections::BTreeMap;

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_anthropic::files::error::FilesError;
use betterlite_config::sections::anthropic_credentials::Settings as ConfigSettings;
use http::header::{HeaderMap, HeaderName, HeaderValue};

use crate::helpers::HelperError;

pub(crate) fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::try_from(name.as_str()).map_err(failure)?;
        let header_value = HeaderValue::from_str(value).map_err(failure)?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

pub(crate) fn settings(
    environment: &BTreeMap<String, String>,
) -> Result<AnthropicSettings, Box<HelperError>> {
    let resolved = ConfigSettings::resolve_environment(environment).map_err(failure)?;
    Ok(AnthropicSettings {
        api_key: resolved.api_key,
        auth_token: resolved.auth_token,
        api_base: resolved.api_base,
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    })
}

pub(crate) fn raised(error: &FilesError) -> serde_json::Value {
    if let FilesError::Provider(provider) = error {
        return serde_json::json!({
            "kind": "status",
            "status": provider.status_code,
            "message": provider.message,
        });
    }
    serde_json::json!({"kind": "ValueError", "message": error.to_string()})
}

pub(crate) fn failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: "Error".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
