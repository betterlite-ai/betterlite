use secrecy::{ExposeSecret, SecretString};

use super::config::{GitLabConfig, ProjectId};
use super::error::GitLabClientError;

pub(super) fn require_credentials(
    config: &GitLabConfig,
) -> Result<(String, &SecretString), GitLabClientError> {
    let (Some(project), Some(access_token)) = (&config.project, &config.access_token) else {
        return Err(GitLabClientError::MissingCredentials);
    };
    if project.is_blank() || access_token.expose_secret().is_empty() {
        return Err(GitLabClientError::MissingCredentials);
    }
    let project = match project {
        ProjectId::Path(path) => path.clone(),
        ProjectId::Number(number) => number.to_string(),
    };
    Ok((project, access_token))
}

mod tests;
