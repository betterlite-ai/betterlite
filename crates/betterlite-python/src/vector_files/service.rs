use std::future::Future;
use std::sync::PoisonError;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_core::api::vector_file_create::service::{
    FileCall as ServiceFileCall, FileService, Operation as FileOperation,
};
use betterlite_core::api::vector_file_http_create_list::config::OpenAiFilesConfig;
use betterlite_core::api::vector_file_http_create_list::error::FileTransportError;
use betterlite_core::api::vector_file_http_create_list::service::{
    CreateCall, FileCall as TransportFileCall, FileDeps, FileTarget, ListCall,
    LogContext as FileLogContext, prepare as prepare_files, send as send_files,
};
use betterlite_core::api::vector_file_http_read::config::{OpenAiReadConfig, ReadConfig};
use betterlite_core::api::vector_file_http_write::config::OpenAiFileWriteConfig;
use betterlite_core::api::vector_file_http_write::error::FileWriteError;
use betterlite_core::api::vector_file_http_write::service::{
    DeleteCall, LogContext as WriteLogContext, UpdateCall, WriteCall as TransportWriteCall,
    WriteDeps, WriteTarget, prepare as prepare_write, send as send_write,
};
use betterlite_core::api::vector_file_read::service::VectorStoreFilesConfigs;
use betterlite_core::api::vector_file_write::service::{
    WriteCall as ServiceWriteCall, WriteService,
};
use betterlite_core::api::vector_store_http_write::service::Operation as WriteOperation;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::SecretString;

use super::session::SessionCore;
use super::source::SilentSink;

// B010's configs for the tags this build serves; every other provider has no vector store file config yet
pub(crate) struct Configs {
    credentials: OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
}

impl Configs {
    pub(crate) const fn new(
        credentials: OpenAiCredentialsSettings,
        globals: LiteLlmGlobals,
        env_api_key: Option<SecretString>,
    ) -> Self {
        Self {
            credentials,
            globals,
            env_api_key,
        }
    }
}

impl VectorStoreFilesConfigs for Configs {
    fn read_config(&self, tag: VectorStoreFilesConfigTag) -> Option<Box<dyn ReadConfig + '_>> {
        match tag {
            VectorStoreFilesConfigTag::OpenAI => Some(Box::new(OpenAiReadConfig::new(
                &self.credentials,
                globals(&self.globals),
                self.env_api_key.clone(),
                self.globals.api_base.clone(),
            ))),
        }
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

// The provider's HTTP create/list service, with B1072's prepared call and its answer
impl FileService for SessionCore {
    fn file(
        &self,
        call: &ServiceFileCall<'_>,
    ) -> impl Future<Output = Result<Json, FileTransportError>> + Send {
        async move {
            let config = OpenAiFilesConfig::new(
                &self.configs.credentials,
                globals(&self.configs.globals),
                self.configs.env_api_key.clone(),
                self.configs.globals.api_base.clone(),
            );
            let target = FileTarget {
                vector_store_id: call.vector_store_id,
                params: call.params,
                extra_headers: call.extra_headers,
                timeout: Some(call.timeout),
            };
            let transport = match call.operation {
                FileOperation::Create => TransportFileCall::Create(CreateCall {
                    target,
                    create_request: call.fields,
                    extra_body: call.extra_body,
                }),
                FileOperation::List => TransportFileCall::List(ListCall {
                    target,
                    query_params: call.fields,
                    extra_query: call.extra_query,
                }),
            };
            let deps = FileDeps {
                config: &config,
                source: &self.source,
                supplied: None,
            };
            let sink = SilentSink;
            let prepared = {
                let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
                let env = self.pre_post(&sink);
                let mut log = FileLogContext {
                    state: &mut state,
                    env: &env,
                };
                prepare_files(&deps, call.mode, &transport, &mut log)?
            };
            send_files(prepared).await
        }
    }
}

// The provider's HTTP write service, with B1074's prepared call, its deadline and its answer
impl WriteService for SessionCore {
    fn write(
        &self,
        call: &ServiceWriteCall<'_>,
    ) -> impl Future<Output = Result<Json, FileWriteError>> + Send {
        async move {
            let config = OpenAiFileWriteConfig {
                settings: &self.configs.credentials,
                globals: globals(&self.configs.globals),
                env_api_key: self.configs.env_api_key.clone(),
                litellm_api_base: self.configs.globals.api_base.clone(),
            };
            let target = WriteTarget {
                vector_store_id: call.vector_store_id,
                file_id: call.file_id,
                params: call.params,
                extra_headers: call.extra_headers,
                timeout: Some(call.timeout),
            };
            let transport = match call.operation {
                WriteOperation::Update => TransportWriteCall::Update(UpdateCall {
                    target,
                    update_request: call.update_request,
                    extra_body: call.extra_body,
                }),
                WriteOperation::Delete => TransportWriteCall::Delete(DeleteCall { target }),
            };
            let deps = WriteDeps {
                config: &config,
                source: &self.source,
                supplied: None,
            };
            let sink = SilentSink;
            let prepared = {
                let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
                let env = self.pre_post(&sink);
                let mut log = WriteLogContext {
                    state: &mut state,
                    env: &env,
                };
                prepare_write(&deps, call.mode, &transport, &mut log)?
            };
            send_write(prepared).await
        }
    }
}
