use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_http::client_kind::ClientKind;
use betterlite_models::decisions::Json;
use betterlite_openai::assistants::client::{ClientOptions, SdkClient};
use betterlite_openai::assistants::operations::AssistantsApi as OpenAiAssistantsApi;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{ThreadCreate, ThreadMessageData};
use secrecy::SecretString;

use super::error::ThreadError;
use super::params::{
    ThreadParams, ThreadParamsError, ThreadTimeout, forwarded_azure_timeout, resolve_timeout,
};
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
        request: Box<ClientRequest<'call>>,
        client: AzureAssistantsClient<'call, T>,
        flag: Option<bool>,
    },
}

pub(super) struct PreparedThread<'call, T> {
    pub(super) target: Target<'call, T>,
    pub(super) operation: Operation,
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
    let converted = metadata
        .map(|value| serde_json::from_value(serde_json::to_value(value)?))
        .transpose()
        .map_err(|source| ThreadParamsError {
            field: None,
            message: source.to_string(),
            source: Some(source),
        })?;
    Ok(ThreadCreate {
        messages,
        metadata: converted,
    })
}

// Duration cannot hold a negative timeout, so a non-positive one becomes the largest deadline
fn deadline(timeout: ThreadTimeout) -> Result<Duration, ThreadError> {
    let seconds = timeout.deadline_seconds();
    if !seconds.is_finite() {
        return Err(ThreadError::Timeout(TimeoutError::NonFinite));
    }
    if seconds <= 0.0_f64 {
        return Ok(Duration::MAX);
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|_source| ThreadError::Timeout(TimeoutError::NonFinite))
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
    let kind = client_kind(flag);
    let options = ClientOptions {
        api_key: resources.openai_api_key(&resolved.options).cloned(),
        api_base: Some(resources.openai_base(&resolved.options)),
        timeout: Some(deadline(resolved.timeout)?),
        max_retries: resolved
            .params
            .max_retries
            .and_then(|value| u32::try_from(value).ok()),
        organization: resources
            .openai_organization(&resolved.options)
            .map(str::to_owned),
    };
    Ok(Target::Openai(
        OpenAiAssistantsApi { transport }.get_openai_client(kind, options, None),
    ))
}

const fn client_kind(flag: Option<bool>) -> ClientKind {
    match flag {
        Some(true) => ClientKind::Asynchronous,
        Some(false) | None => ClientKind::Sync,
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
        let params = ThreadParams::from_kwargs(request.kwargs)?;
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
                let request = Box::new(self.azure_request(resolved, None)?);
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
        let token = super::params::facade_azure_token(self.resources, &resolved.params);
        match client {
            Some(SuppliedClient::Openai(supplied)) if supplied.kind == ClientKind::Asynchronous => {
                return Ok(Target::Openai(supplied));
            }
            Some(SuppliedClient::Azure(supplied)) if supplied.kind == ClientKind::Asynchronous => {
                let request = Box::new(self.azure_request(resolved, token)?);
                return Ok(Target::Azure {
                    request,
                    client: supplied,
                    flag,
                });
            }
            Some(SuppliedClient::Openai(_) | SuppliedClient::Azure(_)) | None => {}
        }
        let request = Box::new(self.azure_request(resolved, token)?);
        let transport = Arc::clone(&self.transport);
        let azure_client =
            AssistantsApi { transport }.azure_client(&request, None, client_kind(flag))?;
        Ok(Target::Azure {
            request,
            client: azure_client,
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
        let request = ClientRequest {
            params: LiteLlmParams {
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
            },
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
        };
        Ok(request)
    }
}
