use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::credential_source::{
    CredentialSource, decode_vertex_credentials,
};
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use serde::Deserialize;

use super::error::CredentialSourceError;
use super::files::FileReader;
use super::sdk::{CredentialBuilders, TokenSource};

pub struct AdcDeps<'deps> {
    pub environment: &'deps AdcEnvironment,
    pub files: &'deps dyn FileReader,
    pub http: &'deps dyn CredentialBuilders,
}

// VertexBase's _credentials_from_default_auth: google.auth.default without the gcloud subprocess
pub async fn credentials_from_default_auth(
    scopes: &[String],
    deps: &AdcDeps<'_>,
) -> Result<(Arc<dyn TokenSource>, Option<String>), CredentialSourceError> {
    let environment = deps.environment;
    let gcloud = environment.gcloud_credentials.as_deref();
    if let Some(path) = environment.application_credentials.as_deref()
        && Some(path) != gcloud
    {
        let source = read_source(path, deps.files)?;
        let handle = build_adc_source(&source, scopes, deps.http)?;
        return Ok((handle, environment.project.clone()));
    }
    if let Some(path) = gcloud {
        match read_optional_source(path, deps.files) {
            Ok(Some(source)) => {
                let handle = build_adc_source(&source, scopes, deps.http)?;
                let project = environment
                    .project
                    .clone()
                    .or_else(|| source.project_id().map(str::to_owned))
                    .or_else(|| environment.gcloud_project.clone())
                    .or_else(|| {
                        gcloud_project(deps.files, environment.gcloud_configuration.as_deref())
                    });
                return Ok((handle, project));
            }
            Ok(None) => {}
            Err(error) => return Err(error),
        }
    }
    let handle = deps
        .http
        .metadata(environment.metadata_host.as_deref(), scopes)?;
    let project = match &environment.project {
        Some(project) => Some(project.clone()),
        None => {
            deps.http
                .metadata_project(environment.metadata_host.as_deref())
                .await
        }
    };
    Ok((handle, project))
}

fn build_adc_source(
    source: &CredentialSource,
    scopes: &[String],
    builders: &dyn CredentialBuilders,
) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
    match source {
        CredentialSource::ServiceAccount(account) => {
            builders.service_account(&account.document(), scopes)
        }
        CredentialSource::AuthorizedUser(user) => {
            builders.authorized_user(&user.document(), scopes)
        }
        CredentialSource::ExternalAccount(account) => {
            let document = account.document();
            if account.aws_environment().is_some() {
                builders.identity_pool_with_aws(&document, scopes)
            } else if account.is_executable() {
                builders.pluggable(&document, scopes)
            } else {
                builders.identity_pool(&document, scopes)
            }
        }
    }
}

fn read_source(
    path: &Path,
    files: &dyn FileReader,
) -> Result<CredentialSource, CredentialSourceError> {
    let bytes = files
        .read(path)
        .map_err(|source| CredentialSourceError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    decode_vertex_credentials(&bytes).map_err(CredentialSourceError::Decode)
}

// the well-known gcloud ADC file only exists after gcloud auth application-default login
fn read_optional_source(
    path: &Path,
    files: &dyn FileReader,
) -> Result<Option<CredentialSource>, CredentialSourceError> {
    match files.read(path) {
        Ok(bytes) => decode_vertex_credentials(&bytes)
            .map(Some)
            .map_err(CredentialSourceError::Decode),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CredentialSourceError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[derive(Debug, Default, Deserialize)]
struct GcloudConfiguration {
    #[serde(default)]
    core: GcloudCore,
}

#[derive(Debug, Default, Deserialize)]
struct GcloudCore {
    project: Option<String>,
}

// `gcloud config get project` runs a subprocess in google-auth; the same value is read from its file
fn gcloud_project(files: &dyn FileReader, path: Option<&Path>) -> Option<String> {
    let bytes = files.read(path?).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let configuration: GcloudConfiguration = serde_saphyr::from_str(text).ok()?;
    configuration
        .core
        .project
        .filter(|project| !project.is_empty())
}
