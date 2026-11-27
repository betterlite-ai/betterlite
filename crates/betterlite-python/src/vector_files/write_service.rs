use std::sync::PoisonError;

use betterlite_core::api::vector_file_http_write::config::OpenAiFileWriteConfig;
use betterlite_core::api::vector_file_http_write::error::FileWriteError;
use betterlite_core::api::vector_file_http_write::service::{
    DeleteCall, LogContext as WriteLogContext, PreparedWrite, UpdateCall,
    WriteCall as TransportWriteCall, WriteDeps, WriteTarget, prepare as prepare_write,
    send as send_write,
};
use betterlite_core::api::vector_file_write::service::{
    WriteCall as ServiceWriteCall, WriteService,
};
use betterlite_core::api::vector_store_http_write::service::Operation as WriteOperation;
use betterlite_models::decisions::Json;

use super::service::globals;
use super::session::SessionCore;
use super::source::{ProxiedSource, SilentSink};

// The provider's HTTP write service, with B1074's prepared call, its deadline and its answer
impl WriteService for SessionCore {
    async fn write(&self, call: &ServiceWriteCall<'_>) -> Result<Json, FileWriteError> {
        let config = OpenAiFileWriteConfig {
            settings: &self.configs.credentials,
            globals: globals(&self.configs.globals),
            env_api_key: self.configs.env_api_key.clone(),
            litellm_api_base: self.configs.globals.api_base.clone(),
        };
        let deps = WriteDeps {
            config: &config,
            source: &self.source,
            supplied: None,
        };
        let prepared = prepared_write(self, &deps, call)?;
        send_write(prepared).await
    }
}

// The prepared update or delete call: its pre-call runs under the session's logging guard, which ends before the send
fn prepared_write<'call>(
    core: &'call SessionCore,
    deps: &'call WriteDeps<'call, OpenAiFileWriteConfig, ProxiedSource>,
    call: &ServiceWriteCall<'_>,
) -> Result<PreparedWrite<'call, OpenAiFileWriteConfig, ProxiedSource>, FileWriteError> {
    let transport = write_transport(call);
    let sink = SilentSink;
    let env = core.pre_post(&sink);
    let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
    let mut log = WriteLogContext {
        state: &mut state,
        env: &env,
    };
    let prepared = prepare_write(deps, call.mode, &transport, &mut log)?;
    drop(state);
    Ok(prepared)
}

// The transport write B1074 sends: the target every write carries, and the request its operation adds
const fn write_transport<'call>(call: &'call ServiceWriteCall<'_>) -> TransportWriteCall<'call> {
    let target = WriteTarget {
        vector_store_id: call.vector_store_id,
        file_id: call.file_id,
        params: call.params,
        extra_headers: call.extra_headers,
        timeout: Some(call.timeout),
    };
    match call.operation {
        WriteOperation::Update => TransportWriteCall::Update(UpdateCall {
            target,
            update_request: call.update_request,
            extra_body: call.extra_body,
        }),
        WriteOperation::Delete => TransportWriteCall::Delete(DeleteCall { target }),
    }
}
