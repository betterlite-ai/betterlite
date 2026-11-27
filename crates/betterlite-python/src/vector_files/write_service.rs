use std::future::Future;
use std::sync::PoisonError;

use betterlite_core::api::vector_file_http_write::config::OpenAiFileWriteConfig;
use betterlite_core::api::vector_file_http_write::error::FileWriteError;
use betterlite_core::api::vector_file_http_write::service::{
    DeleteCall, LogContext as WriteLogContext, UpdateCall, WriteCall as TransportWriteCall,
    WriteDeps, WriteTarget, prepare as prepare_write, send as send_write,
};
use betterlite_core::api::vector_file_write::service::{
    WriteCall as ServiceWriteCall, WriteService,
};
use betterlite_core::api::vector_store_http_write::service::Operation as WriteOperation;
use betterlite_models::decisions::Json;

use super::service::globals;
use super::session::SessionCore;
use super::source::SilentSink;

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
