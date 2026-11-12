use std::io::{self, ErrorKind};
use std::path::Path;
use std::str::from_utf8;
use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::credential_source::{
    CredentialSource, decode_vertex_credentials,
};
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;

use super::error::CredentialSourceError;
use super::files::FileReader;
use super::sdk::{BuiltSource, CredentialBuilders, TokenSource};

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
        let source =
            read_optional_source(path, deps.files)?.ok_or_else(|| CredentialSourceError::Read {
                path: path.to_path_buf(),
                source: io::Error::new(
                    ErrorKind::NotFound,
                    "the vertex credentials file is absent",
                ),
            })?;
        let handle = build_adc_source(&source, scopes, deps.http)?;
        let project = environment
            .project
            .clone()
            .or_else(|| source.project_id().map(str::to_owned));
        return Ok((handle, project));
    }
    if let Some(path) = gcloud
        && let Some(source) = read_optional_source(path, deps.files)?
    {
        let handle = build_adc_source(&source, scopes, deps.http)?;
        let project = environment
            .project
            .clone()
            .or_else(|| source.project_id().map(str::to_owned))
            .or_else(|| environment.gcloud_project.clone())
            .or_else(|| gcloud_project(deps.files, environment.gcloud_configuration.as_deref()));
        return Ok((handle, project));
    }
    metadata_credentials(scopes, deps).await
}

async fn metadata_credentials(
    scopes: &[String],
    deps: &AdcDeps<'_>,
) -> Result<(Arc<dyn TokenSource>, Option<String>), CredentialSourceError> {
    let environment = deps.environment;
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
) -> BuiltSource {
    let document = source.document();
    match source {
        CredentialSource::ServiceAccount(_) => builders.service_account(&document, scopes),
        CredentialSource::AuthorizedUser(_) => builders.authorized_user(&document, scopes),
        CredentialSource::ExternalAccount(account) => {
            if account.aws_environment().is_some() {
                builders.identity_pool_with_aws(&document, scopes)
            } else {
                builders.identity_pool(&document, scopes)
            }
        }
    }
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

// google-auth runs `gcloud config get project`; gcloud keeps that value under [core] in its INI file
fn gcloud_project(files: &dyn FileReader, path: Option<&Path>) -> Option<String> {
    let bytes = files.read(path?).ok()?;
    let core = from_utf8(&bytes)
        .ok()?
        .split("\n[")
        .find_map(|section| section.trim_start_matches('[').strip_prefix("core]"))?;
    core.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(['=', ':'])?;
            (key.trim() == "project").then(|| value.trim().to_owned())
        })
        .filter(|project| !project.is_empty())
}
