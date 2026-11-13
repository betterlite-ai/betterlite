use secrecy::ExposeSecret as _;

use super::credentials::secret_text;
use super::error::ManusResponsesError;
use super::{
    MANUS_API_BASE, MANUS_API_BASE_ENV, ManusResponsesConfig, RESPONSES_SUFFIX, V1_SUFFIX,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResourceRoute {
    Delete,
    Cancel,
}

fn base_url(
    config: &ManusResponsesConfig,
    api_base: Option<&str>,
) -> Result<String, ManusResponsesError> {
    let explicit = api_base.filter(|base| !base.is_empty());
    if let Some(base) = explicit {
        return Ok(base.to_owned());
    }
    if let Some(base) = config
        .globals
        .api_base
        .as_ref()
        .filter(|base| !base.is_empty())
    {
        return Ok(base.clone());
    }
    let environment = secret_text(&config.secrets, &config.environment, MANUS_API_BASE_ENV)?;
    Ok(environment.map_or_else(
        || String::from(MANUS_API_BASE),
        |text| text.expose_secret().to_owned(),
    ))
}

pub(super) fn responses_url(
    config: &ManusResponsesConfig,
    api_base: Option<&str>,
) -> Result<String, ManusResponsesError> {
    let base = base_url(config, api_base)?;
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(V1_SUFFIX) {
        Ok(format!("{trimmed}{RESPONSES_SUFFIX}"))
    } else {
        Ok(format!("{trimmed}{V1_SUFFIX}{RESPONSES_SUFFIX}"))
    }
}
