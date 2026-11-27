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
    LogContext as FileLogContext, PreparedFile, prepare as prepare_files, send as send_files,
};
use betterlite_core::api::vector_file_http_read::config::{OpenAiReadConfig, ReadConfig};
use betterlite_core::api::vector_file_read::service::VectorStoreFilesConfigs;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::SecretString;

use super::session::SessionCore;
use super::source::{ProxiedSource, SilentSink};

// B010's configs for the tags this build serves; every other provider has no vector store file config yet
pub(crate) struct Configs {
    pub(crate) credentials: OpenAiCredentialsSettings,
    pub(crate) globals: LiteLlmGlobals,
    pub(crate) env_api_key: Option<SecretString>,
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
pub(crate) fn globals(source: &LiteLlmGlobals) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: source.api_base.clone(),
        api_key: source.api_key.clone(),
        openai_key: source.openai_key.clone(),
        organization: source.organization.clone(),
    }
}

// The provider's HTTP create/list service, with B1072's prepared call and its answer
impl FileService for SessionCore {
    async fn file(&self, call: &ServiceFileCall<'_>) -> Result<Json, FileTransportError> {
        let config = OpenAiFilesConfig::new(
            &self.configs.credentials,
            globals(&self.configs.globals),
            self.configs.env_api_key.clone(),
            self.configs.globals.api_base.clone(),
        );
        let deps = FileDeps {
            config: &config,
            source: &self.source,
            supplied: None,
        };
        let prepared = prepared_file(self, &deps, call)?;
        send_files(prepared).await
    }
}

// The prepared create or list call: its pre-call runs under the session's logging guard, which ends before the send
fn prepared_file<'call>(
    core: &'call SessionCore,
    deps: &'call FileDeps<'call, OpenAiFilesConfig<'call>, ProxiedSource>,
    call: &ServiceFileCall<'_>,
) -> Result<PreparedFile<'call, OpenAiFilesConfig<'call>, ProxiedSource>, FileTransportError> {
    let transport = file_transport(call);
    let sink = SilentSink;
    let env = core.pre_post(&sink);
    let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
    let mut log = FileLogContext {
        state: &mut state,
        env: &env,
    };
    let prepared = prepare_files(deps, call.mode, &transport, &mut log)?;
    drop(state);
    Ok(prepared)
}

// The transport call B1072 sends: the target every file call carries, and the fields its operation adds
const fn file_transport<'call>(call: &'call ServiceFileCall<'_>) -> TransportFileCall<'call> {
    let target = FileTarget {
        vector_store_id: call.vector_store_id,
        params: call.params,
        extra_headers: call.extra_headers,
        timeout: Some(call.timeout),
    };
    match call.operation {
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
    }
}
