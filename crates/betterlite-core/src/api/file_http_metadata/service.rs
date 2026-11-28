use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::files::records::{FileDeleted, FileObject};
use http::Method;

use self::endpoints::{log_pre_call, request_endpoint, select_client, validate_environment};
use self::listing::{Listing, list_pages};
use self::wire::{file_response, query_pairs, request_url, send_once, status_facts, wire_headers};
use super::config::{FileParams, Headers, MetadataConfig, ReplyContext};
use super::error::{MetadataError, provider_error};
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient, WriteRequest, WriteTransport,
};

mod endpoints;
mod listing;
mod wire;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Retrieve,
    Delete,
    List,
}

impl Operation {
    // LiteLLM hands the per-call timeout to the async delete and to both listing sends, and to no other send
    const fn sends_timeout(self, mode: ClientKind) -> bool {
        matches!(
            (self, mode),
            (Self::Delete, ClientKind::Asynchronous)
                | (Self::List, ClientKind::Sync | ClientKind::Asynchronous)
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct Attempt {
    mode: ClientKind,
    operation: Operation,
}

pub struct MetadataTarget<'call> {
    pub litellm_params: &'call FileParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub ssl_verify: Option<bool>,
    pub timeout: Option<Duration>,
}

pub struct RetrieveCall<'call> {
    pub file_id: &'call str,
    pub target: MetadataTarget<'call>,
}

pub struct DeleteCall<'call> {
    pub file_id: &'call str,
    pub target: MetadataTarget<'call>,
}

pub struct ListCall<'call> {
    pub purpose: Option<&'call str>,
    pub target: MetadataTarget<'call>,
}

pub enum MetadataCall<'call> {
    Retrieve(RetrieveCall<'call>),
    Delete(DeleteCall<'call>),
    List(ListCall<'call>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataValue {
    File(FileObject),
    Deleted(FileDeleted),
    Listed(Vec<FileObject>),
}

pub struct MetadataDeps<'deps, C: MetadataConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

pub struct PreparedMetadata<'deps, C: MetadataConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    attempt: Attempt,
    litellm_params: FileParams,
    headers: Headers,
    request: WriteRequest,
}

// The handler front half in LiteLLM's order: client selection, the transformed request, validation and the pre-call
pub fn prepare<'borrow, 'deps, C, S>(
    deps: &'borrow MetadataDeps<'deps, C, S>,
    mode: ClientKind,
    call: &MetadataCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedMetadata<'borrow, C, S>, MetadataError>
where
    'deps: 'borrow,
    C: MetadataConfig,
    S: ClientSource,
{
    let (operation, target) = match call {
        MetadataCall::Retrieve(retrieve) => (Operation::Retrieve, &retrieve.target),
        MetadataCall::Delete(delete) => (Operation::Delete, &delete.target),
        MetadataCall::List(list) => (Operation::List, &list.target),
    };
    let litellm_params = target.litellm_params;
    let client = select_client(deps, mode, target.ssl_verify)?;
    let optional_params = FileParams::new();
    let (base, query) = request_endpoint(deps.config, call, &optional_params)?;
    let mut headers = Headers::new();
    for (name, value) in target.extra_headers.into_iter().flatten() {
        headers.insert(name.clone(), value.clone());
    }
    let validated = validate_environment(deps.config, headers, &optional_params, litellm_params)?;
    log_pre_call(&base, &validated, call, log)?;
    let timeout = target.timeout.filter(|_| operation.sends_timeout(mode));
    Ok(PreparedMetadata {
        config: deps.config,
        client,
        attempt: Attempt { mode, operation },
        litellm_params: litellm_params.clone(),
        request: WriteRequest {
            method: match operation {
                Operation::Retrieve | Operation::List => Method::GET,
                Operation::Delete => Method::DELETE,
            },
            url: request_url(&base, &query_pairs(&query)?)?,
            headers: wire_headers(&validated)?,
            body: RequestBody::Empty,
            timeout,
        },
        headers: validated,
    })
}

// One send, no retry: a transport failure becomes a provider error; a status of 400 or more only fails a delete, which the client raises
pub async fn send<C, S>(
    prepared: PreparedMetadata<'_, C, S>,
    log: &mut LogContext<'_, '_>,
) -> Result<MetadataValue, MetadataError>
where
    C: MetadataConfig,
    S: ClientSource<Client: WriteTransport>,
{
    let PreparedMetadata {
        config,
        client,
        attempt,
        litellm_params,
        headers,
        request,
    } = prepared;
    let timeout = request.timeout;
    let url = request.url.to_string();
    let reply = send_once(config, client.get(), request, attempt).await?;
    match attempt.operation {
        Operation::Retrieve => {
            let response = file_response(&reply, &url);
            let file = config.transform_retrieve_file_response(
                &response,
                ReplyContext {
                    model_call_details: &mut log.state.model_call_details,
                    litellm_params: &litellm_params,
                },
            )?;
            Ok(MetadataValue::File(file))
        }
        Operation::Delete => deleted(config, &reply, &url, log, &litellm_params),
        Operation::List => {
            let listing = Listing {
                config,
                client: client.get(),
                attempt,
                litellm_params,
                headers,
                timeout,
            };
            list_pages(&listing, log, reply, url)
                .await
                .map(MetadataValue::Listed)
        }
    }
}

// The delete client raises on a status of 400 or more before its transform sees the reply
fn deleted(
    config: &dyn MetadataConfig,
    reply: &UpstreamResponse,
    url: &str,
    log: &mut LogContext<'_, '_>,
    litellm_params: &FileParams,
) -> Result<MetadataValue, MetadataError> {
    if reply.status >= 400 {
        let facts = status_facts(reply);
        return Err(MetadataError::Provider(provider_error(&facts, config)));
    }
    let response = file_response(reply, url);
    let deleted = config.transform_delete_file_response(
        &response,
        ReplyContext {
            model_call_details: &mut log.state.model_call_details,
            litellm_params,
        },
    )?;
    Ok(MetadataValue::Deleted(deleted))
}
