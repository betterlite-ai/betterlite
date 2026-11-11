use secrecy::SecretString;

use crate::cli_context::CliContext;
use crate::config::normalize_base_url;

use super::args::GroupConnection;
use super::error::ConfigureCommandError;

pub fn connection_settings(
    root: &CliContext,
    group: Option<&GroupConnection>,
    api_key: Option<&SecretString>,
    gateway_url: Option<&str>,
) -> Result<CliContext, ConfigureCommandError> {
    let key = api_key.or_else(|| {
        let options = group?;
        options.api_key.as_ref()
    });
    let url = gateway_url.or_else(|| {
        let options = group?;
        options.gateway_url.as_deref()
    });
    let normalized = normalize_base_url(url.unwrap_or(&root.base_url))?;
    Ok(CliContext {
        base_url: normalized
            .strip_suffix("/v1")
            .unwrap_or(&normalized)
            .to_owned(),
        base_url_explicit: url.is_some() || root.base_url_explicit,
        api_key: key.cloned().or_else(|| root.api_key.clone()),
        api_key_from_token_file: key.is_none() && root.api_key_from_token_file,
    })
}
