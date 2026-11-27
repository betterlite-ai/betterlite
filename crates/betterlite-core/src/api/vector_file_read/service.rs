use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoreFilesConfigTag, select_store_files_config,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::vector_stores::Headers;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::VectorFileReadError;
use crate::api::vector_file_http_read::config::{QueryParams, ReadConfig, ReadParams};
use crate::api::vector_file_http_read::error::PrepareFailure;
use crate::api::vector_file_http_read::service::{
    ClientSource, Operation, ReadDeps, ReadTarget, prepare, send,
};
use crate::api::vector_file_provider::{
    CredentialLookup, OperationKwargs, ensure_provider, prepare_registry_credentials,
};
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use crate::api::vector_store_http_write::service::{ClientKind, LogContext, SuppliedClient};

// The wrapper's named arguments and the call's **kwargs
pub struct ReadCall<'call> {
    pub vector_store_id: &'call str,
    pub file_id: &'call str,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'call str>,
    pub kwargs: &'call JsonObject,
}

// The operation and handler family the wrapper's flag selected, over the kwargs the pop left
pub struct ReadEntry<'call> {
    pub operation: Operation,
    pub mode: ClientKind,
    pub kwargs: &'call JsonObject,
}

// The provider configs B010 selected, which the caller builds for the tags it serves
pub trait VectorStoreFilesConfigs: Send + Sync {
    fn read_config(&self, tag: VectorStoreFilesConfigTag) -> Option<Box<dyn ReadConfig + '_>>;
}

// B2481 takes any sized config, so a config the caller built behind a box is lent as one
pub struct OwnedReadConfig<'config> {
    inner: Box<dyn ReadConfig + 'config>,
}

impl ReadConfig for OwnedReadConfig<'_> {
    fn validate_environment(&self, headers: Headers, params: &ReadParams) -> Headers {
        self.inner.validate_environment(headers, params)
    }

    fn complete_url(
        &self,
        params: &ReadParams,
        vector_store_id: &str,
    ) -> Result<String, PrepareFailure> {
        self.inner.complete_url(params, vector_store_id)
    }

    fn transform_retrieve_request(
        &self,
        file_id: &str,
        api_base: &str,
    ) -> Result<(String, QueryParams), PrepareFailure> {
        self.inner.transform_retrieve_request(file_id, api_base)
    }

    fn transform_content_request(
        &self,
        file_id: &str,
        api_base: &str,
    ) -> Result<(String, QueryParams), PrepareFailure> {
        self.inner.transform_content_request(file_id, api_base)
    }

    fn transform_retrieve_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        self.inner.transform_retrieve_response(response)
    }

    fn transform_content_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        self.inner.transform_content_response(response)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        self.inner.error_class(message, status_code, headers)
    }
}

pub struct SdkDeps<'deps, S: ClientSource> {
    pub configs: &'deps dyn VectorStoreFilesConfigs,
    pub registry: Option<&'deps dyn CredentialLookup>,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
    pub request_timeout_seconds: f64,
}

// kwargs.pop("aretrieve") is read as `is True`: a 1, a truthy string or a false selects the sync handler
#[must_use]
pub fn entry_from_flag(kwargs: &JsonObject, flag: &str) -> ClientKind {
    if matches!(kwargs.get(flag), Some(Json::Bool(true))) {
        return ClientKind::Asynchronous;
    }
    ClientKind::Sync
}

// The arguments the read handler receives: the two ids, GenericLiteLLMParams, the caller's headers and the wait the GET never uses
pub(super) struct HandlerCall<'call> {
    pub(super) vector_store_id: &'call str,
    pub(super) file_id: &'call str,
    pub(super) params: ReadParams,
    pub(super) extra_headers: Option<&'call BTreeMap<String, String>>,
    pub(super) timeout: Duration,
    pub(super) mode: ClientKind,
    pub(super) operation: Operation,
}

// After the pop: the credential overlay, GenericLiteLLMParams, the provider's config, the log and B2481's read
pub(super) async fn read_attachment<S>(
    deps: &SdkDeps<'_, S>,
    call: &ReadCall<'_>,
    log: &mut LogContext<'_, '_>,
    entry: &ReadEntry<'_>,
) -> Result<Json, VectorFileReadError>
where
    S: ClientSource,
{
    let call_id = entry
        .kwargs
        .get("litellm_call_id")
        .cloned()
        .unwrap_or(Json::Null);
    let mut overlaid = OperationKwargs(entry.kwargs.clone());
    prepare_registry_credentials(call.vector_store_id, &mut overlaid, deps.registry);
    let params = read_params(&overlaid.0)?;
    let provider = ensure_provider(call.custom_llm_provider).to_owned();
    let tag = files_config_tag(&provider, entry.operation)?;
    log_read(
        log.state,
        call,
        &LogFields {
            kwargs: &overlaid.0,
            call_id: &call_id,
            provider: &provider,
        },
    )?;
    let handler = handler_call(call, params, entry, deps.request_timeout_seconds)?;
    let inner = deps
        .configs
        .read_config(tag)
        .ok_or_else(|| VectorFileReadError::Unsupported {
            operation: operation_label(entry.operation),
            provider: provider.clone(),
        })?;
    read_send(deps, log, provider, OwnedReadConfig { inner }, &handler).await
}

// timeout or request_timeout: a null or zero argument uses the setting, and any other value is the wait in seconds
pub(super) fn handler_call<'call>(
    call: &'call ReadCall<'_>,
    params: ReadParams,
    entry: &ReadEntry<'_>,
    request_timeout_seconds: f64,
) -> Result<HandlerCall<'call>, VectorFileReadError> {
    let seconds = call
        .timeout
        .filter(|value| *value != 0.0_f64)
        .unwrap_or(request_timeout_seconds);
    let timeout = Duration::try_from_secs_f64(seconds).map_err(VectorFileReadError::Timeout)?;
    Ok(HandlerCall {
        vector_store_id: call.vector_store_id,
        file_id: call.file_id,
        params,
        extra_headers: call.extra_headers,
        timeout,
        mode: entry.mode,
        operation: entry.operation,
    })
}

async fn read_send<S>(
    deps: &SdkDeps<'_, S>,
    log: &mut LogContext<'_, '_>,
    provider: String,
    config: OwnedReadConfig<'_>,
    handler: &HandlerCall<'_>,
) -> Result<Json, VectorFileReadError>
where
    S: ClientSource,
{
    let target = ReadTarget {
        vector_store_id: handler.vector_store_id,
        file_id: handler.file_id,
        params: &handler.params,
        extra_headers: handler.extra_headers,
        timeout: Some(handler.timeout),
    };
    let supplied = deps.supplied.as_ref().map(|held| SuppliedClient {
        kind: held.kind,
        transport: held.transport,
    });
    let read_deps = ReadDeps {
        config: &config,
        source: deps.source,
        supplied,
    };
    let prepared =
        prepare(&read_deps, handler.mode, handler.operation, &target, log).map_err(|source| {
            VectorFileReadError::Read {
                provider: provider.clone(),
                source,
            }
        })?;
    send(prepared)
        .await
        .map_err(|source| VectorFileReadError::Read { provider, source })
}

// GenericLiteLLMParams' three fields the read handlers read
fn read_params(kwargs: &JsonObject) -> Result<ReadParams, VectorFileReadError> {
    Ok(ReadParams {
        api_base: text(kwargs, "api_base")?,
        api_key: text(kwargs, "api_key")?.map(SecretString::from),
        ssl_verify: flag(kwargs, "ssl_verify")?,
    })
}

// LlmProviders(custom_llm_provider) and B010's files config, the two ValueErrors the wrapper raises before any HTTP
fn files_config_tag(
    provider: &str,
    operation: Operation,
) -> Result<VectorStoreFilesConfigTag, VectorFileReadError> {
    let parsed = provider.parse::<ProviderId>().map_err(|_unparsed| {
        VectorFileReadError::InvalidProvider {
            name: provider.to_owned(),
        }
    })?;
    select_store_files_config(parsed).ok_or_else(|| VectorFileReadError::Unsupported {
        operation: operation_label(operation),
        provider: provider.to_owned(),
    })
}

const fn operation_label(operation: Operation) -> &'static str {
    match operation {
        Operation::Retrieve => "retrieve",
        Operation::Content => "content retrieve",
    }
}

fn text(kwargs: &JsonObject, field: &'static str) -> Result<Option<String>, VectorFileReadError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VectorFileReadError::Field {
                field,
                expected: "a string",
            })
        }
    }
}

fn flag(kwargs: &JsonObject, field: &'static str) -> Result<Option<bool>, VectorFileReadError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VectorFileReadError::Field {
                field,
                expected: "a boolean",
            })
        }
    }
}

struct LogFields<'fields> {
    kwargs: &'fields JsonObject,
    call_id: &'fields Json,
    provider: &'fields str,
}

// update_from_kwargs(model=None, optional_params={ids}, litellm_params={vector_store_id, litellm_call_id, **model_dump})
fn log_read(
    state: &mut LoggingState,
    call: &ReadCall<'_>,
    fields: &LogFields<'_>,
) -> Result<(), VectorFileReadError> {
    let optional_params = IndexMap::from([
        (
            "vector_store_id".to_owned(),
            Json::Text(call.vector_store_id.to_owned()),
        ),
        ("file_id".to_owned(), Json::Text(call.file_id.to_owned())),
    ]);
    let mut litellm_params = IndexMap::from([
        (
            "vector_store_id".to_owned(),
            Json::Text(call.vector_store_id.to_owned()),
        ),
        ("litellm_call_id".to_owned(), fields.call_id.clone()),
    ]);
    litellm_params.extend(generic_params_dump(fields.kwargs));
    let additional_params = IndexMap::from([(
        "custom_llm_provider".to_owned(),
        Json::Text(fields.provider.to_owned()),
    )]);
    update_from_kwargs(
        state,
        fields.kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )?;
    Ok(())
}

// GenericLiteLLMParams.model_dump(exclude_none=True): the five fields defaulting to False, then every non-null kwarg
fn generic_params_dump(kwargs: &JsonObject) -> IndexMap<String, Json> {
    let mut dump: IndexMap<String, Json> = [
        "allow_client_keepalive_override",
        "merge_reasoning_content_in_choices",
        "use_in_pass_through",
        "use_litellm_proxy",
        "use_xai_oauth",
    ]
    .into_iter()
    .map(|name| (name.to_owned(), Json::Bool(false)))
    .collect();
    dump.extend(
        kwargs
            .iter()
            .filter(|(_, value)| !matches!(value, Json::Null))
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    dump
}
