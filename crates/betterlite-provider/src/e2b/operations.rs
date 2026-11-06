use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::sandbox::{
    CodeExecutionResult, ContainerHandle, HiddenParams, SandboxOutputError, read_capped_lines,
};
use http::header::{CONTENT_TYPE, HeaderName};
use http::{HeaderMap, HeaderValue, Method};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use url::Url;

use super::parse::parse_ndjson;
use super::{
    CreateSandbox, DEFAULT_SANDBOX_TIMEOUT, DeleteSandbox, E2B_API_BASE, E2B_DEFAULT_DOMAIN,
    E2B_DEFAULT_TEMPLATE, E2bError, E2bSandbox, JUPYTER_PORT, RunCode, SandboxHttp, SandboxRef,
    SandboxRequest, check_status, secret_header_value,
};

const E2B_API_KEY_VARIABLE: &str = "E2B_API_KEY";

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CreateBody {
    #[serde(rename = "templateID")]
    pub(crate) template_id: String,
    pub(crate) timeout: i64,
    pub(crate) secure: bool,
    pub(crate) allow_internet_access: bool,
}

#[derive(Debug, Deserialize)]
struct CreateResponse {
    #[serde(rename = "sandboxID")]
    sandbox_id: String,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default, rename = "envdAccessToken")]
    envd_access_token: Option<SecretString>,
    #[serde(default, rename = "trafficAccessToken")]
    traffic_access_token: Option<SecretString>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RunBody {
    pub(crate) code: String,
    pub(crate) context_id: Option<String>,
    pub(crate) env_vars: Option<BTreeMap<String, String>>,
}

impl<Http, Environment> E2bSandbox<Http, Environment>
where
    Http: SandboxHttp,
    Environment: EnvironmentLookup + Sync,
{
    pub fn validate_environment(&self, api_key: Option<&str>) -> Result<SecretString, E2bError> {
        api_key.map_or_else(
            || {
                self.environment
                    .lookup(E2B_API_KEY_VARIABLE)
                    .filter(|key| !key.is_empty())
                    .map(SecretString::from)
                    .ok_or(E2bError::MissingApiKey)
            },
            |key| Ok(SecretString::from(key)),
        )
    }

    pub async fn acreate_sandbox(
        &self,
        request: CreateSandbox<'_>,
    ) -> Result<ContainerHandle, E2bError> {
        let api_key = self.validate_environment(request.api_key)?;
        let api_base = request.api_base.unwrap_or(E2B_API_BASE);
        let body = create_body(&request);
        let sent = self
            .http
            .send_buffered(create_request(api_base, &api_key, &body)?)
            .await?;
        check_status(sent.status)?;
        created_handle(&sent.body, api_key, api_base)
    }

    pub async fn arun_code(&self, request: RunCode<'_>) -> Result<CodeExecutionResult, E2bError> {
        let sent = self.http.send_streaming(run_request(&request)?).await?;
        check_status(sent.status)?;
        let lines = read_capped_lines(sent.body).await.map_err(output_error)?;
        Ok(parse_ndjson(&lines))
    }

    pub async fn adelete_sandbox(&self, request: DeleteSandbox<'_>) -> Result<bool, E2bError> {
        let handle = as_handle(request.container);
        let api_key = match (request.api_key, handle.hidden_params.api_key.clone()) {
            (Some(api_key), _) => SecretString::from(api_key),
            (None, Some(stored)) => stored,
            (None, None) => self.validate_environment(None)?,
        };
        let api_base = request
            .api_base
            .map(str::to_owned)
            .or_else(|| handle.hidden_params.api_base.clone())
            .unwrap_or_else(|| E2B_API_BASE.to_owned());
        let sent = self
            .http
            .send_streaming(delete_request(&handle, &api_key, &api_base)?)
            .await?;
        if sent.status == 404 {
            return Ok(false);
        }
        check_status(sent.status)?;
        Ok((200..300).contains(&sent.status))
    }
}

fn create_body(request: &CreateSandbox<'_>) -> CreateBody {
    CreateBody {
        template_id: request.template.unwrap_or(E2B_DEFAULT_TEMPLATE).to_owned(),
        timeout: request.timeout.unwrap_or(DEFAULT_SANDBOX_TIMEOUT),
        secure: true,
        allow_internet_access: request.allow_internet_access.unwrap_or(true),
    }
}

fn create_request(
    api_base: &str,
    api_key: &SecretString,
    body: &CreateBody,
) -> Result<SandboxRequest, E2bError> {
    let url =
        Url::parse(&format!("{api_base}/sandboxes")).map_err(|source| E2bError::Url { source })?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("x-api-key"),
        secret_header_value(api_key)?,
    );
    let encoded = serde_json::to_vec(body).map_err(|source| E2bError::Encode { source })?;
    Ok(SandboxRequest {
        method: Method::POST,
        url,
        headers,
        body: encoded,
    })
}

fn created_handle(
    body: &[u8],
    api_key: SecretString,
    api_base: &str,
) -> Result<ContainerHandle, E2bError> {
    let data: CreateResponse =
        serde_json::from_slice(body).map_err(|source| E2bError::CreateResponse { source })?;
    Ok(ContainerHandle {
        id: data.sandbox_id,
        provider: "e2b".to_owned(),
        domain: Some(data.domain.unwrap_or_else(|| E2B_DEFAULT_DOMAIN.to_owned())),
        hidden_params: HiddenParams {
            envd_access_token: data.envd_access_token,
            traffic_access_token: data.traffic_access_token,
            api_key: Some(api_key),
            api_base: Some(api_base.to_owned()),
        },
    })
}

fn run_request(request: &RunCode<'_>) -> Result<SandboxRequest, E2bError> {
    let handle = request.container;
    let token = handle
        .hidden_params
        .envd_access_token
        .as_ref()
        .ok_or(E2bError::MissingAccessToken)?;
    let domain = handle.domain.as_deref().unwrap_or(E2B_DEFAULT_DOMAIN);
    let url = Url::parse(&format!(
        "https://{JUPYTER_PORT}-{}.{domain}/execute",
        handle.id
    ))
    .map_err(|source| E2bError::Url { source })?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("x-access-token"),
        secret_header_value(token)?,
    );
    if let Some(traffic) = &handle.hidden_params.traffic_access_token {
        headers.insert(
            HeaderName::from_static("e2b-traffic-access-token"),
            secret_header_value(traffic)?,
        );
    }
    let body = RunBody {
        code: request.code.to_owned(),
        context_id: None,
        env_vars: request.env_vars.cloned(),
    };
    let encoded = serde_json::to_vec(&body).map_err(|source| E2bError::Encode { source })?;
    Ok(SandboxRequest {
        method: Method::POST,
        url,
        headers,
        body: encoded,
    })
}

fn delete_request(
    handle: &ContainerHandle,
    api_key: &SecretString,
    api_base: &str,
) -> Result<SandboxRequest, E2bError> {
    let url = Url::parse(&format!("{api_base}/sandboxes/{}", handle.id))
        .map_err(|source| E2bError::Url { source })?;
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-api-key"),
        secret_header_value(api_key)?,
    );
    Ok(SandboxRequest {
        method: Method::DELETE,
        url,
        headers,
        body: Vec::new(),
    })
}

fn as_handle(container: SandboxRef<'_>) -> ContainerHandle {
    match container {
        SandboxRef::Handle(handle) => handle.clone(),
        SandboxRef::Id(id) => ContainerHandle {
            id: id.to_owned(),
            provider: "e2b".to_owned(),
            domain: Some(E2B_DEFAULT_DOMAIN.to_owned()),
            hidden_params: HiddenParams::default(),
        },
    }
}

fn output_error(error: SandboxOutputError<E2bError>) -> E2bError {
    match error {
        SandboxOutputError::Exceeded { limit } => E2bError::OutputTooLarge { limit },
        SandboxOutputError::Utf8 { source } => E2bError::OutputNotUtf8 { source },
        SandboxOutputError::Read { source } => source,
    }
}
