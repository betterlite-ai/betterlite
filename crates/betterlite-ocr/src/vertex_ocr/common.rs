use std::iter::once;

use secrecy::{ExposeSecret as _, SecretString};

use crate::json::{Json, Object};
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_config::connection::{OcrConnection, OcrInputSource};
use crate::ocr_config::has_header;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::OcrClient;

use super::VertexOcrEnvironment;

pub(crate) const VERTEX_AI_API_KEY: &str = "VERTEX_AI_API_KEY";
pub(crate) const DEFAULT_LOCATION: &str = "us-central1";
pub(crate) const SECRET_NAMES: [&str; 7] = [
    "VERTEX_AI_API_KEY",
    "VERTEXAI_API_KEY",
    "VERTEXAI_CREDENTIALS",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "VERTEXAI_PROJECT",
    "VERTEXAI_LOCATION",
    "VERTEX_LOCATION",
];
const VERTEX_PROJECT_OPTIONS: [&str; 2] = ["vertex_project", "vertex_ai_project"];
const VERTEX_LOCATION_OPTIONS: [&str; 2] = ["vertex_location", "vertex_ai_location"];

pub(crate) struct VertexConfig {
    pub(crate) project: Option<String>,
    pub(crate) location: Option<String>,
}

pub(crate) fn vertex_config(request: &PreparedOcrRequest) -> Result<VertexConfig, OcrError> {
    let params = &request.optional_params;
    let settings = &request.connection.settings;
    Ok(VertexConfig {
        project: optional_string(params, &VERTEX_PROJECT_OPTIONS)?
            .or_else(|| configured(settings.vertex_project.as_deref())),
        location: optional_string(params, &VERTEX_LOCATION_OPTIONS)?
            .or_else(|| configured(settings.vertex_location.as_deref())),
    })
}

pub(crate) fn project(request: &PreparedOcrRequest, config: &VertexConfig) -> Option<String> {
    config
        .project
        .clone()
        .or_else(|| non_empty_secret(&request.connection, "VERTEXAI_PROJECT"))
}

pub(crate) fn location(request: &PreparedOcrRequest, config: &VertexConfig) -> String {
    config
        .location
        .clone()
        .or_else(|| non_empty_secret(&request.connection, "VERTEXAI_LOCATION"))
        .or_else(|| non_empty_secret(&request.connection, "VERTEX_LOCATION"))
        .unwrap_or_else(|| DEFAULT_LOCATION.to_owned())
}

pub(crate) async fn environment(
    request: &PreparedOcrRequest,
    client: &OcrClient,
) -> Result<VertexOcrEnvironment, OcrError> {
    let config = vertex_config(request)?;
    validate_destination(&request.connection)?;
    let named_project = project(request, &config);
    let extra_headers = request.connection.extra_headers.clone();
    if has_header(&extra_headers, "authorization") {
        let project_id = match named_project {
            Some(named) => named,
            None => client
                .vertex
                .token(request)
                .await?
                .project
                .ok_or_else(missing_project)?,
        };
        return Ok(VertexOcrEnvironment {
            headers: extra_headers,
            project_id,
        });
    }
    let (token, project_id) = access_token(
        request,
        client,
        static_token(&request.connection),
        named_project,
    )
    .await?;
    Ok(VertexOcrEnvironment {
        headers: bearer_headers(&token, &extra_headers)?,
        project_id,
    })
}

pub(crate) fn validate_destination(connection: &OcrConnection) -> Result<(), OcrError> {
    if connection.api_base.is_some() && connection.api_base_source == OcrInputSource::Request {
        return Err(OcrError::Auth(OcrAuthError::InvalidConfiguration {
            detail: "credentials cannot be sent to a request-controlled Vertex AI endpoint"
                .to_owned(),
        }));
    }
    Ok(())
}

async fn access_token(
    request: &PreparedOcrRequest,
    client: &OcrClient,
    static_token: Option<SecretString>,
    project: Option<String>,
) -> Result<(SecretString, String), OcrError> {
    if let (Some(held), Some(named)) = (&static_token, &project) {
        return Ok((held.clone(), named.clone()));
    }
    let access = client.vertex.token(request).await?;
    let token = static_token.unwrap_or(access.token);
    let project_id = project.or(access.project).ok_or_else(missing_project)?;
    Ok((token, project_id))
}

// a caller-supplied authorization header replaces both the static key and the credential token
fn bearer_headers(
    token: &SecretString,
    extra_headers: &[(String, String)],
) -> Result<Vec<(String, String)>, OcrError> {
    let credential = token.expose_secret().trim();
    if credential.is_empty() {
        return Err(OcrError::Auth(OcrAuthError::InvalidConfiguration {
            detail: "the Vertex AI access token is empty".to_owned(),
        }));
    }
    Ok(
        once(("Authorization".to_owned(), format!("Bearer {credential}")))
            .chain(extra_headers.iter().cloned())
            .collect(),
    )
}

fn static_token(connection: &OcrConnection) -> Option<SecretString> {
    connection
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().trim().is_empty())
        .or_else(|| secret_token(connection, "VERTEX_AI_API_KEY"))
        .or_else(|| secret_token(connection, "VERTEXAI_API_KEY"))
}

fn secret_token(connection: &OcrConnection, name: &str) -> Option<SecretString> {
    connection
        .secrets
        .get(name)
        .map(|value| SecretString::from(value.expose_secret().trim()))
        .filter(|value| !value.expose_secret().is_empty())
}

fn non_empty_secret(connection: &OcrConnection, name: &str) -> Option<String> {
    connection
        .secrets
        .get(name)
        .map(|value| value.expose_secret().trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn optional_string(params: &Object, names: &[&str]) -> Result<Option<String>, OcrError> {
    for name in names {
        match params.get(*name) {
            Some(Json::Text(value)) if !value.trim().is_empty() => {
                return Ok(Some(value.clone()));
            }
            None | Some(Json::Null | Json::Text(_)) => {}
            Some(_) => {
                return Err(OcrError::RequestField {
                    path: (*name).to_owned(),
                });
            }
        }
    }
    Ok(None)
}

// LiteLLM's or_configured drops an empty configured value and keeps a blank one
fn configured(value: Option<&str>) -> Option<String> {
    match value {
        Some(named) if !named.is_empty() => Some(named.to_owned()),
        Some(_) | None => None,
    }
}

fn missing_project() -> OcrError {
    OcrError::Auth(OcrAuthError::InvalidConfiguration {
        detail: "vertex_project is required: pass vertex_project, set VERTEXAI_PROJECT or use credentials with a project"
            .to_owned(),
    })
}
