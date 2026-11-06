use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::sandbox::{
    ContainerHandle, HiddenParams, SandboxOutputError, read_capped_lines,
};
use http::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::value::RawValue;
use url::Url;

use super::constants::{API_BASE_VARIABLE, API_KEY_VARIABLE};
use super::reply::check_status;
use super::sse::raw_text;
use super::{OpenSandbox, OpenSandboxError, RequestTimeout, SandboxHttp, SandboxRequest};

const EXECUTION_TIMEOUT: RequestTimeout = RequestTimeout {
    connect: Some(Duration::from_secs(30)),
    read: None,
    write: Some(Duration::from_secs(30)),
    pool: None,
};

pub struct PostCode<'request> {
    pub url: &'request str,
    pub headers: HeaderMap,
    pub body: PostCodeBody,
}

#[derive(Debug, Serialize)]
pub struct PostCodeBody {
    pub code: String,
    pub context: ExecutionContext,
}

#[derive(Debug, Serialize)]
pub struct ExecutionContext {
    pub language: String,
}

#[derive(Debug, Clone, Copy)]
pub enum SandboxRef<'request> {
    Handle(&'request ContainerHandle),
    Id(&'request str),
}

impl<Http, Environment, Clock> OpenSandbox<Http, Environment, Clock>
where
    Environment: EnvironmentLookup + Sync,
{
    #[must_use]
    pub fn validate_environment(&self, api_key: Option<&SecretString>) -> SecretString {
        api_key.map_or_else(
            || {
                SecretString::from(
                    self.environment
                        .lookup(API_KEY_VARIABLE)
                        .filter(|key| !key.is_empty())
                        .unwrap_or_default(),
                )
            },
            Clone::clone,
        )
    }

    pub fn api_base(&self, api_base: Option<&str>) -> Result<String, OpenSandboxError> {
        let candidate = api_base.map_or_else(
            || {
                self.environment
                    .lookup(API_BASE_VARIABLE)
                    .unwrap_or_default()
            },
            <str>::to_owned,
        );
        if candidate.is_empty() {
            return Err(OpenSandboxError::MissingApiBase {
                variable: API_BASE_VARIABLE,
            });
        }
        Ok(candidate.trim_end_matches('/').to_owned())
    }

    #[must_use]
    pub fn api_key(
        &self,
        api_key: Option<&SecretString>,
        handle: &ContainerHandle,
    ) -> SecretString {
        if let Some(key) = api_key {
            return key.clone();
        }
        let stored = handle.hidden_params.api_key.as_ref();
        stored.map_or_else(|| self.validate_environment(None), Clone::clone)
    }

    pub fn as_handle<'request>(
        &self,
        container: SandboxRef<'request>,
        api_base: Option<&str>,
    ) -> Result<Cow<'request, ContainerHandle>, OpenSandboxError> {
        match container {
            SandboxRef::Handle(handle) => Ok(Cow::Borrowed(handle)),
            SandboxRef::Id(id) => Ok(Cow::Owned(ContainerHandle {
                id: id.to_owned(),
                provider: "opensandbox".to_owned(),
                domain: Some(self.api_base(api_base)?),
                hidden_params: HiddenParams::default(),
            })),
        }
    }
}

impl<Http, Environment, Clock> OpenSandbox<Http, Environment, Clock>
where
    Http: SandboxHttp,
    Clock: Sync,
    Environment: Sync,
{
    pub async fn post_code(
        &self,
        request: PostCode<'_>,
        client: Option<&Http>,
    ) -> Result<Vec<String>, OpenSandboxError> {
        let url = Url::parse(request.url).map_err(|source| OpenSandboxError::Url { source })?;
        let body = serde_json::to_vec(&request.body)
            .map_err(|source| OpenSandboxError::Encode { source })?;
        let sent = SandboxRequest {
            method: Method::POST,
            url,
            headers: request.headers,
            body,
            timeout: Some(EXECUTION_TIMEOUT),
        };
        let response = client.unwrap_or(&self.http).send(sent).await?;
        check_status(response.status)?;
        read_capped_lines(response.body)
            .await
            .map_err(|error| match error {
                SandboxOutputError::Exceeded { limit } => {
                    OpenSandboxError::OutputTooLarge { limit }
                }
                SandboxOutputError::Utf8 { source } => OpenSandboxError::OutputNotUtf8 { source },
                SandboxOutputError::Read { source } => source,
            })
    }
}

pub fn lifecycle_headers(api_key: &SecretString) -> Result<HeaderMap, OpenSandboxError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if !api_key.expose_secret().is_empty() {
        let mut value = HeaderValue::try_from(api_key.expose_secret())
            .map_err(|source| OpenSandboxError::InvalidHeader { source })?;
        value.set_sensitive(true);
        headers.insert(HeaderName::from_static("open-sandbox-api-key"), value);
    }
    Ok(headers)
}

#[must_use]
pub fn endpoint_base_url(endpoint: &str, api_base: &str) -> String {
    let normalized = endpoint.trim_end_matches('/');
    if normalized.starts_with("http://") || normalized.starts_with("https://") {
        return normalized.to_owned();
    }
    let protocol = api_base
        .split_once("://")
        .map_or("http", |(protocol, _)| protocol);
    format!("{protocol}://{normalized}")
}

#[must_use]
pub fn as_str_dict(value: Option<&BTreeMap<String, Box<RawValue>>>) -> BTreeMap<String, String> {
    value.map_or_else(BTreeMap::new, |fields| {
        fields
            .iter()
            .map(|(key, raw)| (key.clone(), raw_text(raw)))
            .collect()
    })
}
