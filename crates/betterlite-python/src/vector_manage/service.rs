use std::sync::PoisonError;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_core::api::vector_store_http_create_read::config::{
    CreateReadConfig, OpenAiCreateReadConfig,
};
use betterlite_core::api::vector_store_http_create_read::error::CreateReadError;
use betterlite_core::api::vector_store_http_create_read::service::{
    CreateReadCall, CreateReadDeps, CreateReadTarget, ListCall as HttpListCall,
    ListQuery as HttpListQuery, RetrieveCall, prepare as prepare_read, send as send_read,
};
use betterlite_core::api::vector_store_http_write::config::OpenAiWriteConfig;
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, WriteError,
};
use betterlite_core::api::vector_store_http_write::service::{
    DeleteCall, LogContext, SuppliedClient, UpdateCall, WriteCall as HttpWriteCall, WriteDeps,
    WriteTarget, prepare as prepare_write, send as send_write,
};
use betterlite_core::api::vector_store_read::service::{
    Operation as ReadOperation, ReadCall, ReadConfig, ReadProvider, VectorStoreReadConfigs,
};
use betterlite_core::api::vector_store_search::provider::ResolvedProvider;
use betterlite_core::api::vector_store_write::service::{
    Operation as WriteOperation, WriteCall as CoreWriteCall, WriteService,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::ports::WrapperClock;
use secrecy::SecretString;

use super::source::{ProxiedSource, SilentSink};

mod tests;

// The credentials and clients one session's calls are served from: B010's read config, B2485's read provider and B2402's write service
pub(crate) struct SessionService<'core> {
    pub(crate) credentials: &'core OpenAiCredentialsSettings,
    pub(crate) globals: &'core LiteLlmGlobals,
    pub(crate) env_api_key: &'core Option<SecretString>,
    pub(crate) source: &'core ProxiedSource,
    pub(crate) redactor: &'core Redactor,
    pub(crate) clock: &'core dyn WrapperClock,
}

impl VectorStoreReadConfigs for SessionService<'_> {
    fn read_config<'source>(
        &'source self,
        resolved: &ResolvedProvider,
    ) -> Option<ReadConfig<'source>> {
        // B010 serves OpenAI's vector stores only
        if resolved.tag != VectorStoresConfigTag::OpenAI {
            return None;
        }
        let config: Box<dyn CreateReadConfig + 'source> = Box::new(OpenAiCreateReadConfig::new(
            self.credentials,
            globals(self.globals),
            self.env_api_key.clone(),
            self.globals.api_base.clone(),
        ));
        Some(ReadConfig(config))
    }
}

impl ReadProvider for SessionService<'_> {
    type Client = ProxiedClient;

    async fn read(&self, call: &ReadCall<'_, ProxiedClient>) -> Result<Json, CreateReadError> {
        let deps = CreateReadDeps {
            config: &call.config,
            source: self.source,
            supplied: call.client.map(|supplied| SuppliedClient {
                kind: supplied.kind,
                transport: supplied.transport,
            }),
        };
        let http_call = read_call(call)?;
        let sink = SilentSink;
        let env = pre_post(self.redactor, &sink, self.clock);
        let prepared = {
            let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
            prepare_read(
                &deps,
                call.mode,
                &http_call,
                &mut LogContext {
                    state: &mut state,
                    env: &env,
                },
            )
        };
        send_read(prepared?).await
    }
}

// B2485's HTTP call for a read: retrieve's id, or list's four cursor fields over the same target
fn read_call<'call>(
    call: &ReadCall<'call, ProxiedClient>,
) -> Result<CreateReadCall<'call>, CreateReadError> {
    let target = CreateReadTarget {
        params: call.params,
        extra_headers: call.extra_headers,
        timeout: Some(call.timeout),
    };
    match call.operation {
        ReadOperation::Retrieve => {
            let Some(vector_store_id) = call.vector_store_id else {
                // the read core only builds a retrieve with its id, so this guards a broken caller
                return Err(missing_id());
            };
            Ok(CreateReadCall::Retrieve(RetrieveCall {
                target,
                vector_store_id,
                extra_body: call.extra_body,
            }))
        }
        ReadOperation::List => Ok(CreateReadCall::List(HttpListCall {
            target,
            query: HttpListQuery {
                after: call.query.after,
                before: call.query.before,
                limit: call.query.limit,
                order: call.query.order,
            },
            extra_body: call.extra_body,
        })),
    }
}

fn missing_id() -> CreateReadError {
    CreateReadError::Write(WriteError::Provider(BaseLlmException {
        status_code: 500,
        message: String::from("a retrieve read needs a vector store id"),
        headers: HeaderMap::new(),
        status_code_is_synthesized: true,
    }))
}

impl WriteService for SessionService<'_> {
    async fn write(&self, call: &CoreWriteCall<'_>) -> Result<Json, WriteError> {
        // B010 serves OpenAI's vector stores only; a tag with no config here names its provider back
        if call.config != VectorStoresConfigTag::OpenAI {
            return Err(unsupported(call));
        }
        let config = OpenAiWriteConfig::new(
            self.credentials,
            globals(self.globals),
            self.env_api_key.clone(),
            self.globals.api_base.clone(),
        );
        let deps = WriteDeps {
            config: &config,
            source: self.source,
            supplied: None,
        };
        let write = write_call(call);
        let sink = SilentSink;
        let env = pre_post(self.redactor, &sink, self.clock);
        let prepared = {
            let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
            prepare_write(
                &deps,
                call.mode,
                &write,
                &mut LogContext {
                    state: &mut state,
                    env: &env,
                },
            )
        };
        send_write(prepared?).await
    }
}

// B2402's HTTP call for a write: update's optional fields, or delete's bare target
fn write_call<'call>(call: &CoreWriteCall<'call>) -> HttpWriteCall<'call> {
    let target = WriteTarget {
        vector_store_id: call.vector_store_id,
        params: call.params,
        extra_headers: call.extra_headers,
        timeout: Some(call.timeout),
    };
    match call.operation {
        WriteOperation::Update => HttpWriteCall::Update(UpdateCall {
            target,
            optional_params: call.optional_params,
            extra_body: call.extra_body,
        }),
        WriteOperation::Delete => HttpWriteCall::Delete(DeleteCall {
            target,
            extra_body: call.extra_body,
        }),
    }
}

// the local refusal of a provider whose vector stores this build has no config for
fn unsupported(call: &CoreWriteCall<'_>) -> WriteError {
    WriteError::Provider(BaseLlmException {
        status_code: 400,
        message: format!(
            "Vector store {} is not supported for {}",
            call.operation.label(),
            call.provider
        ),
        headers: HeaderMap::new(),
        status_code_is_synthesized: false,
    })
}

// The Pre Call environment a Python session starts from where the SDK's decorator would have built it
fn pre_post<'env>(
    redactor: &'env Redactor,
    sink: &'env SilentSink,
    clock: &dyn WrapperClock,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: clock.now(),
    }
}

// B527 owns the credential source rules, so the globals are copied into the shape its resolver reads
fn globals(source: &LiteLlmGlobals) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: source.api_base.clone(),
        api_key: source.api_key.clone(),
        openai_key: source.openai_key.clone(),
        organization: source.organization.clone(),
    }
}
