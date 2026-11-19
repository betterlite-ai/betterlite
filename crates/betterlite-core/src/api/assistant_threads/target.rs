use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_models::decisions::Json;
use betterlite_openai::assistants::client::{ClientKind, ClientOptions, SdkClient};
use betterlite_openai::assistants::operations::AssistantsApi as OpenAiAssistantsApi;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{ThreadCreate, ThreadMessageData};
use secrecy::SecretString;

use super::error::ThreadError;
use super::params::{ThreadParams, ThreadTimeout, forwarded_azure_timeout, resolve_timeout};
use super::{AssistantThreads, SuppliedClient, ThreadOperation};
use crate::api::resource_sdk_settings::timeout::TimeoutError;
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

pub(super) enum Operation {
    Create(ThreadCreate),
    Retrieve(String),
}

pub(super) enum Target<'call, T> {
    Openai(SdkClient<T>),
    Azure {
        request: ClientRequest<'call>,
        client: AzureAssistantsClient<'call, T>,
        flag: Option<bool>,
    },
}

pub(super) struct PreparedThread<'call, T> {
    pub(super) target: Target<'call, T>,
    pub(super) operation: Operation,
    pub(super) timeout: ThreadTimeout,
}

pub(super) struct PrepareRequest<'call, T> {
    pub(super) provider: String,
    pub(super) operation: Operation,
    pub(super) client: Option<SuppliedClient<'call, T>>,
    pub(super) kwargs: &'call BTreeMap<String, Json>,
    pub(super) facade: ThreadOperation,
    pub(super) flag: Option<bool>,
}

struct Resolved {
    params: ThreadParams,
    options: ResourceSdkRequestOptions,
    timeout: ThreadTimeout,
}

// B520 reads the typed metadata map; the caller's untagged JSON converts into it without changing any value
pub(super) fn create_body(
    messages: Option<Vec<ThreadMessageData>>,
    metadata: Option<BTreeMap<String, Json>>,
) -> Result<ThreadCreate, ThreadError> {
    let metadata: Option<_> = metadata
        .map(|metadata| {
            serde_json::to_value(metadata).and_then(|value| serde_json::from_value(value))
        })
        .transpose()
        .map_err(|source| ThreadError::Params {
            field: None,
            message: source.to_string(),
        })?;
    Ok(ThreadCreate { messages, metadata })
}

// httpx reads a zero or negative timeout as no timeout; this transport takes the largest deadline instead
fn deadline(timeout: ThreadTimeout) -> Result<Duration, ThreadError> {
    let seconds = timeout.deadline_seconds();
    if !seconds.is_finite() {
        return Err(ThreadError::Timeout(TimeoutError::NonFinite));
    }
    if seconds <= 0.0 {
        return Ok(Duration::MAX);
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|_source| ThreadError::Timeout(TimeoutError::NonFinite))
}

// the OpenAI client's options from B2487's shared chain and the caller's kwargs
fn openai_options(
    resources: &ResourceSdkSettings,
    params: &ThreadParams,
    options: &ResourceSdkRequestOptions,
    timeout: ThreadTimeout,
) -> Result<ClientOptions, ThreadError> {
    Ok(ClientOptions {
        api_key: resources.openai_api_key(options).cloned(),
        api_base: Some(resources.openai_base(options)),
        timeout: Some(deadline(timeout)?),
        max_retries: params
            .max_retries
            .and_then(|value| u32::try_from(value).ok()),
        organization: resources.openai_organization(options).map(str::to_owned),
    })
}

// B176's litellm_params: the caller's azure_ad_token, the auth fields and the forwarded timeout
fn azure_params(params: &ThreadParams) -> Result<LiteLlmParams, ThreadError> {
    Ok(LiteLlmParams {
        azure_ad_token: params.azure_ad_token.clone(),
        auth: AuthParams {
            tenant_id: params.tenant_id.clone(),
            client_id: params.client_id.clone(),
            client_secret: params.client_secret.clone(),
            azure_username: params.azure_username.clone(),
            azure_password: params.azure_password.clone(),
            azure_scope: params.azure_scope.clone(),
        },
        max_retries: params.max_retries,
        timeout: forwarded_azure_timeout(params)?,
    })
}

fn openai_client<'call, T>(
    transport: Arc<T>,
    resources: &ResourceSdkSettings,
    resolved: &Resolved,
    flag: Option<bool>,
) -> Result<Target<'call, T>, ThreadError>
where
    T: AssistantsTransport,
{
    let options = openai_options(
        resources,
        &resolved.params,
        &resolved.options,
        resolved.timeout,
    )?;
    Ok(Target::Openai(
        OpenAiAssistantsApi { transport }.get_openai_client(client_kind(flag), options, None),
    ))
}

const fn client_kind(flag: Option<bool>) -> ClientKind {
    match flag {
        Some(true) => ClientKind::Async,
        Some(false) | None => ClientKind::Sync,
    }
}

// absent extra_body pops nothing, an explicit null reads AZURE_AD_TOKEN, and the body's token is popped and wins
fn facade_azure_token(
    resources: &ResourceSdkSettings,
    params: &ThreadParams,
) -> Option<SecretString> {
    match &params.extra_body {
        None => None,
        Some(None) => resources.azure_ad_token.clone(),
        Some(Some(body)) => match body.get("azure_ad_token") {
            Some(Json::Text(token)) => Some(SecretString::from(token.clone())),
            Some(_) | None => None,
        },
    }
}

impl<'deps, T> AssistantThreads<'deps, T>
where
    T: AssistantsTransport,
{
    pub(super) fn prepare<'call>(
        &self,
        request: PrepareRequest<'call, T>,
    ) -> Result<PreparedThread<'call, T>, ThreadError>
    where
        'deps: 'call,
    {
        let params =
            ThreadParams::from_kwargs(request.kwargs).map_err(|error| ThreadError::Params {
                field: error.field,
                message: error.message,
            })?;
        let timeout = resolve_timeout(&params)?;
        let options = ResourceSdkRequestOptions {
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            api_key: params.api_key.clone(),
            organization: params.organization.clone(),
            vertex_project: None,
            vertex_location: None,
            vertex_credentials: None,
        };
        let resolved = Resolved {
            params,
            options,
            timeout,
        };
        let target = match request.provider.as_str() {
            "openai" => self.openai_target(&resolved, request.client, request.flag)?,
            "azure" => self.azure_target(&resolved, request.client, request.flag)?,
            other => {
                return Err(ThreadError::Unsupported {
                    provider: other.to_owned(),
                    operation: request.facade.name(),
                });
            }
        };
        Ok(PreparedThread {
            target,
            operation: request.operation,
            timeout,
        })
    }

    // OpenAIAssistantsAPI.create_thread and get_thread: a supplied client wins untouched
    fn openai_target<'call>(
        &self,
        resolved: &Resolved,
        client: Option<SuppliedClient<'call, T>>,
        flag: Option<bool>,
    ) -> Result<Target<'call, T>, ThreadError>
    where
        'deps: 'call,
    {
        match client {
            Some(SuppliedClient::Openai(supplied)) => Ok(Target::Openai(supplied)),
            Some(SuppliedClient::Azure(supplied)) => {
                let request = self.azure_request(resolved, None)?;
                Ok(Target::Azure {
                    request,
                    client: supplied,
                    flag,
                })
            }
            None => openai_client(Arc::clone(&self.transport), self.resources, resolved, flag),
        }
    }

    // AzureAssistantsAPI.create_thread and get_thread: isinstance(client, OpenAI) drops the sync classes only
    fn azure_target<'call>(
        &self,
        resolved: &Resolved,
        client: Option<SuppliedClient<'call, T>>,
        flag: Option<bool>,
    ) -> Result<Target<'call, T>, ThreadError>
    where
        'deps: 'call,
    {
        let token = facade_azure_token(self.resources, &resolved.params);
        match client {
            Some(SuppliedClient::Openai(supplied)) if supplied.kind == ClientKind::Async => {
                return Ok(Target::Openai(supplied));
            }
            Some(SuppliedClient::Azure(supplied)) if supplied.kind == ClientKind::Async => {
                let request = self.azure_request(resolved, token)?;
                return Ok(Target::Azure {
                    request,
                    client: supplied,
                    flag,
                });
            }
            Some(SuppliedClient::Openai(_) | SuppliedClient::Azure(_)) | None => {}
        }
        let request = self.azure_request(resolved, token)?;
        let transport = Arc::clone(&self.transport);
        let client = AssistantsApi { transport }
            .azure_client(&request, None, client_kind(flag))
            .map_err(|error| ThreadError::Call {
                message: error.to_string(),
            })?;
        Ok(Target::Azure {
            request,
            client,
            flag,
        })
    }

    fn azure_request<'call>(
        &self,
        resolved: &Resolved,
        token: Option<SecretString>,
    ) -> Result<ClientRequest<'call>, ThreadError>
    where
        'deps: 'call,
    {
        let params = &resolved.params;
        Ok(ClientRequest {
            params: azure_params(params)?,
            provider: None,
            api_key: self.resources.azure_api_key(&resolved.options).cloned(),
            api_base: self
                .resources
                .azure_base(&resolved.options)
                .map(str::to_owned),
            api_version: self
                .resources
                .azure_version(&resolved.options)
                .map(str::to_owned),
            // the facade's own azure_ad_token: accepted for signature parity, never forwarded
            azure_ad_token: token,
            timeout: resolved.timeout.deadline_seconds(),
            max_retries: params.max_retries,
            refresh_enabled: self.refresh_enabled,
            session: self.session,
            settings: self.azure,
            environment: self.environment,
            sources: self.sources,
        })
    }
}
