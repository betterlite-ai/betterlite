use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::sandbox::{
    CodeExecutionResult, ContainerHandle, HiddenParams, read_capped_lines,
};
use http::header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use serde::Deserialize;
use serde_json::value::RawValue;
use url::Url;

use super::super::helpers::constants::{DEFAULT_LANGUAGE, POLL_INTERVAL, READY_TIMEOUT};
use super::super::helpers::create::{CreateSandbox, create_body};
use super::super::helpers::poll::{
    Endpoint, GetEndpoint, WaitForEndpoint, WaitUntilRunning, sandbox_state,
};
use super::super::helpers::request::{
    ExecutionContext, PostCode, PostCodeBody, SandboxRef, endpoint_base_url, lifecycle_headers,
};
use super::super::helpers::sse::parse_lines;
use super::super::helpers::{
    OpenSandbox, OpenSandboxError, PollClock, SandboxHttp, SandboxRequest, check_status,
};
use super::{ContainerRef, CreateSandboxRequest, LifecycleError, RunCode, SandboxHandle};

#[derive(Debug, Deserialize)]
struct CreatedSandbox {
    id: String,
}

struct Created {
    id: String,
    running: bool,
}

#[derive(Debug, Clone, Copy)]
struct Waiting<'request> {
    api_base: &'request str,
    headers: &'request HeaderMap,
    use_server_proxy: bool,
    ready_timeout: Duration,
    poll_interval: Duration,
}

impl<Http, Environment, Clock> OpenSandbox<Http, Environment, Clock>
where
    Http: SandboxHttp,
    Environment: EnvironmentLookup + Sync,
    Clock: PollClock,
{
    pub async fn acreate_sandbox(
        &self,
        request: CreateSandboxRequest<'_>,
        client: Option<&Http>,
    ) -> Result<SandboxHandle, LifecycleError> {
        let api_key = self.validate_environment(request.api_key);
        let api_base = self.api_base(request.api_base)?;
        let headers = lifecycle_headers(&api_key)?;
        let use_server_proxy = request.use_server_proxy;
        let ready = Waiting {
            api_base: &api_base,
            headers: &headers,
            use_server_proxy,
            ready_timeout: request.ready_timeout.unwrap_or(READY_TIMEOUT),
            poll_interval: request.poll_interval.unwrap_or(POLL_INTERVAL),
        };
        let created = self.create_sandbox(&request, &ready, client).await?;
        let endpoint = self.await_ready(&created, &ready, client).await?;
        Ok(SandboxHandle {
            container: ContainerHandle {
                id: created.id,
                provider: "opensandbox".to_owned(),
                domain: Some(api_base.clone()),
                hidden_params: HiddenParams {
                    envd_access_token: None,
                    traffic_access_token: None,
                    api_key: Some(api_key),
                    api_base: Some(api_base),
                },
            },
            execd_endpoint: endpoint,
            use_server_proxy,
        })
    }

    pub async fn arun_code(
        &self,
        request: RunCode<'_>,
        client: Option<&Http>,
    ) -> Result<CodeExecutionResult, LifecycleError> {
        let handle = self.ensure_handle(&request, client).await?;
        let api_base = match handle
            .container
            .hidden_params
            .api_base
            .as_deref()
            .or(handle.container.domain.as_deref())
        {
            Some(base) => base.to_owned(),
            None => self.api_base(request.api_base)?,
        };
        let url = format!(
            "{}/code",
            endpoint_base_url(&handle.execd_endpoint.url, &api_base)
        );
        let headers = execution_headers(&handle.execd_endpoint.headers)?;
        let language = request.language.unwrap_or(DEFAULT_LANGUAGE);
        let lines = self
            .post_code(
                PostCode {
                    url: &url,
                    headers,
                    body: PostCodeBody {
                        code: request.code.to_owned(),
                        context: ExecutionContext {
                            language: language.to_owned(),
                        },
                    },
                },
                client,
            )
            .await?;
        Ok(parse_lines(&lines))
    }

    pub(super) async fn ensure_handle(
        &self,
        request: &RunCode<'_>,
        client: Option<&Http>,
    ) -> Result<SandboxHandle, LifecycleError> {
        let container: Cow<'_, ContainerHandle> = match request.container {
            ContainerRef::Cached(handle) => return Ok(handle.clone()),
            ContainerRef::Handle(handle) => Cow::Borrowed(handle),
            ContainerRef::Id(id) => self.as_handle(SandboxRef::Id(id), request.api_base)?,
        };
        let api_base = match container.hidden_params.api_base.as_deref() {
            Some(base) => base.to_owned(),
            None => self.api_base(request.api_base)?,
        };
        let api_key = self.api_key(request.api_key, &container);
        let headers = lifecycle_headers(&api_key)?;
        let endpoint = self
            .wait_for_execd_endpoint(
                WaitForEndpoint {
                    endpoint: GetEndpoint {
                        sandbox_id: &container.id,
                        api_base: &api_base,
                        headers: &headers,
                        use_server_proxy: request.use_server_proxy,
                    },
                    ready_timeout: request.ready_timeout.unwrap_or(READY_TIMEOUT),
                    poll_interval: request.poll_interval.unwrap_or(POLL_INTERVAL),
                },
                client,
            )
            .await?;
        let mut resolved = container.into_owned();
        resolved.domain = Some(api_base.clone());
        resolved.hidden_params.api_base = Some(api_base);
        resolved.hidden_params.api_key = Some(api_key);
        Ok(SandboxHandle {
            container: resolved,
            execd_endpoint: endpoint,
            use_server_proxy: request.use_server_proxy,
        })
    }

    async fn create_sandbox(
        &self,
        request: &CreateSandboxRequest<'_>,
        ready: &Waiting<'_>,
        client: Option<&Http>,
    ) -> Result<Created, LifecycleError> {
        let body = create_body(&CreateSandbox {
            template: request.template,
            timeout: request.timeout,
            allow_internet_access: request.allow_internet_access,
            metadata: request.metadata,
            env_vars: request.env_vars,
            resource_limits: request.resource_limits,
            resource_requests: request.resource_requests,
            entrypoint: request.entrypoint,
            network_policy: request.network_policy,
            secure_access: request.secure_access,
        });
        let encoded =
            serde_json::to_vec(&body).map_err(|source| OpenSandboxError::Encode { source })?;
        let url = Url::parse(&format!("{}/sandboxes", ready.api_base))
            .map_err(|source| OpenSandboxError::Url { source })?;
        let response = client
            .unwrap_or(&self.http)
            .send(SandboxRequest {
                method: Method::POST,
                url,
                headers: ready.headers.clone(),
                body: encoded,
                timeout: None,
            })
            .await?;
        check_status(response.status)?;
        let lines = read_capped_lines(response.body).await?;
        let document: Box<RawValue> = serde_json::from_str(&lines.join("\n"))
            .map_err(|source| OpenSandboxError::Decode { source })?;
        let created: CreatedSandbox = serde_json::from_str(document.get())
            .map_err(|source| OpenSandboxError::Decode { source })?;
        Ok(Created {
            id: created.id,
            running: sandbox_state(&document).as_deref() == Some("Running"),
        })
    }

    async fn await_ready(
        &self,
        created: &Created,
        ready: &Waiting<'_>,
        client: Option<&Http>,
    ) -> Result<Endpoint, LifecycleError> {
        if !created.running {
            self.wait_until_running(
                &WaitUntilRunning {
                    sandbox_id: &created.id,
                    api_base: ready.api_base,
                    headers: ready.headers,
                    ready_timeout: ready.ready_timeout,
                    poll_interval: ready.poll_interval,
                },
                client,
            )
            .await?;
        }
        let endpoint = self
            .wait_for_execd_endpoint(
                WaitForEndpoint {
                    endpoint: GetEndpoint {
                        sandbox_id: &created.id,
                        api_base: ready.api_base,
                        headers: ready.headers,
                        use_server_proxy: ready.use_server_proxy,
                    },
                    ready_timeout: ready.ready_timeout,
                    poll_interval: ready.poll_interval,
                },
                client,
            )
            .await?;
        Ok(endpoint)
    }
}

fn execution_headers(
    endpoint_headers: &BTreeMap<String, String>,
) -> Result<HeaderMap, LifecycleError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    for (name, value) in endpoint_headers {
        let header_name = HeaderName::try_from(name.as_str())
            .map_err(|source| LifecycleError::InvalidHeaderName { source })?;
        let header_value = HeaderValue::try_from(value.as_str())
            .map_err(|source| OpenSandboxError::InvalidHeader { source })?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}
