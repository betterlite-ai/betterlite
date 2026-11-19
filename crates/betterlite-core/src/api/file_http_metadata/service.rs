use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{FileDeleted, FileObject};
use http::Method;

use self::endpoints::{
    log_pre_call, pre_call_marker, request_endpoint, select_client, split_call,
    validate_environment,
};
use self::listing::{Listing, list_pages};
use self::wire::{
    delete_response, file_response, query_pairs, request_url, send_once, timeout_for, wire_headers,
};
use super::config::{FileLogging, FileParams, Headers, MetadataConfig};
use super::error::MetadataError;
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient, WriteRequest,
};

mod endpoints;
mod facts;
mod listing;
mod wire;

const NO_MESSAGES: [Json; 0] = [];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Retrieve,
    Delete,
    List,
}

impl Operation {
    const fn method(self) -> Method {
        match self {
            Self::Retrieve | Self::List => Method::GET,
            Self::Delete => Method::DELETE,
        }
    }

    // LiteLLM hands the per-call timeout to the async delete and to both listing sends, and to no other send
    const fn sends_timeout(self, mode: ClientKind) -> bool {
        match (self, mode) {
            (Self::Delete, ClientKind::Asynchronous)
            | (Self::List, ClientKind::Sync)
            | (Self::List, ClientKind::Asynchronous) => true,
            (Self::Retrieve, ClientKind::Sync)
            | (Self::Retrieve, ClientKind::Asynchronous)
            | (Self::Delete, ClientKind::Sync) => false,
        }
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
    mode: ClientKind,
    operation: Operation,
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
    let (operation, target) = split_call(call);
    let client = select_client(deps, mode, target.ssl_verify)?;
    let optional_params = FileParams::new();
    let (base, query) = request_endpoint(deps.config, call, &optional_params)?;
    let headers = validate_environment(deps.config, target, &optional_params)?;
    log_pre_call(&base, &headers, pre_call_marker(call), log)?;
    Ok(PreparedMetadata {
        config: deps.config,
        client,
        mode,
        operation,
        litellm_params: target.litellm_params.clone(),
        request: WriteRequest {
            method: operation.method(),
            url: request_url(&base, &query_pairs(&query)?)?,
            headers: wire_headers(&headers)?,
            body: RequestBody::Empty,
            timeout: timeout_for(operation, mode, target.timeout),
        },
        headers,
    })
}

// One send, no retry: a transport failure becomes a provider error; a status of 400 or more only fails a delete, which the client raises
pub async fn send<C, S>(
    prepared: PreparedMetadata<'_, C, S>,
    log: &mut LogContext<'_, '_>,
) -> Result<MetadataValue, MetadataError>
where
    C: MetadataConfig,
    S: ClientSource,
{
    let PreparedMetadata {
        config,
        client,
        mode,
        operation,
        litellm_params,
        headers,
        request,
    } = prepared;
    let timeout = request.timeout;
    let first_url = request.url.to_string();
    let attempt = Attempt { mode, operation };
    let reply = send_once(config, client.get(), request, attempt).await?;
    match operation {
        Operation::Retrieve => {
            let response = file_response(&reply, &first_url);
            let file = config.transform_retrieve_file_response(
                &response,
                &mut FileLogging {
                    model_call_details: &mut log.state.model_call_details,
                },
                &litellm_params,
            )?;
            Ok(MetadataValue::File(file))
        }
        Operation::Delete => {
            let response = delete_response(config, &reply, &first_url)?;
            let deleted = config.transform_delete_file_response(
                &response,
                &mut FileLogging {
                    model_call_details: &mut log.state.model_call_details,
                },
                &litellm_params,
            )?;
            Ok(MetadataValue::Deleted(deleted))
        }
        Operation::List => {
            let mut listing = Listing {
                config,
                client: client.get(),
                mode,
                litellm_params,
                headers,
                timeout,
            };
            list_pages(&mut listing, log, reply, first_url)
                .await
                .map(MetadataValue::Listed)
        }
    }
}
