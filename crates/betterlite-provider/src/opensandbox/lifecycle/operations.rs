use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::sandbox::{
    CodeExecutionResult, ContainerHandle, HiddenParams, read_capped_lines,
};
use http::header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::value::RawValue;
use url::Url;

use super::super::helpers::constants::{DEFAULT_LANGUAGE, POLL_INTERVAL, READY_TIMEOUT};
use super::super::helpers::create::{CreateSandbox, create_body};
use super::super::helpers::poll::{GetEndpoint, WaitForEndpoint, WaitUntilRunning, sandbox_state};
use super::super::helpers::request::{
    ExecutionContext, PostCode, PostCodeBody, SandboxRef, endpoint_base_url, lifecycle_headers,
};
use super::super::helpers::sse::parse_lines;
use super::super::helpers::{
    OpenSandbox, OpenSandboxError, PollClock, SandboxHttp, SandboxRequest, check_status,
};
use super::{
    ContainerRef, CreateSandboxRequest, LifecycleError, RunCode, SandboxHandle, SandboxOptions,
};

#[derive(Debug, Deserialize)]
struct CreatedSandbox {
    id: String,
}

struct Created {
    container: ContainerHandle,
    running: bool,
    waiting: Waiting,
}

struct Waiting {
    api_key: SecretString,
    api_base: String,
    headers: HeaderMap,
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
        let created = self.create_sandbox(&request, client).await?;
        self.ready_handle(created.container, &created.waiting, created.running, client)
            .await
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
            None => self.api_base(request.options.api_base)?,
        };
        let url = format!(
            "{}/code",
            endpoint_base_url(&handle.execd_endpoint, &api_base)
        );
        let headers = execution_headers(&handle.execd_headers)?;
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
        let container = match request.container {
            ContainerRef::Cached(handle) => return Ok(handle.clone()),
            ContainerRef::Handle(handle) => handle.clone(),
            ContainerRef::Id(id) => self
                .as_handle(SandboxRef::Id(id), request.options.api_base)?
                .into_owned(),
        };
        let api_base = match container.hidden_params.api_base.clone() {
            Some(base) => base,
            None => self.api_base(request.options.api_base)?,
        };
        let api_key = self.api_key(request.options.api_key, &container);
        let waiting = prepare_waiting(api_key, api_base, request.options)?;
        self.ready_handle(container, &waiting, true, client).await
    }

    async fn ready_handle(
        &self,
        mut container: ContainerHandle,
        waiting: &Waiting,
        running: bool,
        client: Option<&Http>,
    ) -> Result<SandboxHandle, LifecycleError> {
        let (running_request, endpoint_request) = wait_requests(&container.id, waiting);
        if !running {
            self.wait_until_running(&running_request, client).await?;
        }
        let endpoint = self
            .wait_for_execd_endpoint(endpoint_request, client)
            .await?;
        container.domain = Some(waiting.api_base.clone());
        container.hidden_params.api_base = Some(waiting.api_base.clone());
        container.hidden_params.api_key = Some(waiting.api_key.clone());
        Ok(SandboxHandle {
            container,
            execd_endpoint: endpoint.url,
            execd_headers: secret_headers(&endpoint.headers),
            use_server_proxy: waiting.use_server_proxy,
        })
    }

    async fn create_sandbox(
        &self,
        request: &CreateSandboxRequest<'_>,
        client: Option<&Http>,
    ) -> Result<Created, LifecycleError> {
        let api_key = self.validate_environment(request.options.api_key);
        let api_base = self.api_base(request.options.api_base)?;
        let waiting = prepare_waiting(api_key, api_base, request.options)?;
        let response = client
            .unwrap_or(&self.http)
            .send(create_request(request, &waiting)?)
            .await?;
        check_status(response.status)?;
        let lines = read_capped_lines(response.body).await?;
        let document: Box<RawValue> = serde_json::from_str(&lines.join("\n"))
            .map_err(|source| OpenSandboxError::Decode { source })?;
        let created: CreatedSandbox = serde_json::from_str(document.get())
            .map_err(|source| OpenSandboxError::Decode { source })?;
        let container = ContainerHandle {
            id: created.id,
            provider: "opensandbox".to_owned(),
            domain: None,
            hidden_params: HiddenParams {
                envd_access_token: None,
                traffic_access_token: None,
                api_key: None,
                api_base: None,
            },
        };
        Ok(Created {
            container,
            running: sandbox_state(&document).as_deref() == Some("Running"),
            waiting,
        })
    }
}

fn prepare_waiting(
    api_key: SecretString,
    api_base: String,
    options: SandboxOptions<'_>,
) -> Result<Waiting, LifecycleError> {
    Ok(Waiting {
        headers: lifecycle_headers(&api_key)?,
        api_key,
        api_base,
        use_server_proxy: options.use_server_proxy,
        ready_timeout: options.ready_timeout.unwrap_or(READY_TIMEOUT),
        poll_interval: options.poll_interval.unwrap_or(POLL_INTERVAL),
    })
}

fn wait_requests<'wait>(
    sandbox_id: &'wait str,
    waiting: &'wait Waiting,
) -> (WaitUntilRunning<'wait>, WaitForEndpoint<'wait>) {
    let running = WaitUntilRunning {
        sandbox_id,
        api_base: &waiting.api_base,
        headers: &waiting.headers,
        ready_timeout: waiting.ready_timeout,
        poll_interval: waiting.poll_interval,
    };
    let endpoint = WaitForEndpoint {
        endpoint: GetEndpoint {
            sandbox_id,
            api_base: &waiting.api_base,
            headers: &waiting.headers,
            use_server_proxy: waiting.use_server_proxy,
        },
        ready_timeout: waiting.ready_timeout,
        poll_interval: waiting.poll_interval,
    };
    (running, endpoint)
}

fn create_request(
    request: &CreateSandboxRequest<'_>,
    waiting: &Waiting,
) -> Result<SandboxRequest, LifecycleError> {
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
    let url = Url::parse(&format!("{}/sandboxes", waiting.api_base))
        .map_err(|source| OpenSandboxError::Url { source })?;
    Ok(SandboxRequest {
        method: Method::POST,
        url,
        headers: waiting.headers.clone(),
        body: encoded,
        timeout: None,
    })
}

fn secret_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, SecretString> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect()
}

fn execution_headers(
    endpoint_headers: &BTreeMap<String, SecretString>,
) -> Result<HeaderMap, LifecycleError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    for (name, value) in endpoint_headers {
        let header_name = HeaderName::try_from(name.as_str())
            .map_err(|source| LifecycleError::InvalidHeaderName { source })?;
        let mut header_value = HeaderValue::try_from(value.expose_secret())
            .map_err(|source| OpenSandboxError::InvalidHeader { source })?;
        header_value.set_sensitive(true);
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}
