use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_file_http_write::config::{OpenAiFileWriteConfig, WriteParams};
use betterlite_core::api::vector_file_http_write::error::FileWriteError;
use betterlite_core::api::vector_file_http_write::service::{
    DeleteCall, UpdateCall, WriteCall, WriteDeps, WriteTarget, send,
};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, source};
use super::{Call, Failure, Operation, Outcome};

// The production service over the mock's base: the same config sections and pooled client a request would build
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    // The OpenAI provider config over the env's key, exactly as the proxy wires B527
    let provider = OpenAiFileWriteConfig::new(
        &config.sections.openai_credentials,
        LiteLlmGlobals::default(),
        config.sections.resource_sdk_settings.openai_api_key.clone(),
        None,
    );
    let origin = source(&config, env)?;
    let mut fixture = Fixture::new(call.logging_call_type())?;
    let params = WriteParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        ssl_verify: None,
    };
    let write = write_call(call, &params);
    let deps = WriteDeps {
        config: &provider,
        source: &origin,
        supplied: None,
    };
    let prepared = fixture.prepare(&deps, call.mode.client_kind(), &write)?;
    let answer = outcome(send(prepared).await)?;
    Ok(SdkResult::VectorFileWriteTransport {
        value: Box::new(answer),
    })
}

fn write_call<'call>(call: &'call Call, params: &'call WriteParams) -> WriteCall<'call> {
    let target = WriteTarget {
        vector_store_id: &call.vector_store_id,
        file_id: &call.file_id,
        params,
        extra_headers: call.extra_headers.as_ref(),
        timeout: call.timeout_seconds.map(Duration::from_secs),
    };
    match call.operation {
        Operation::Update => WriteCall::Update(UpdateCall {
            target,
            update_request: &call.update_request,
            extra_body: call.extra_body.as_ref(),
        }),
        Operation::Delete => WriteCall::Delete(DeleteCall { target }),
    }
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(result: Result<Json, FileWriteError>) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(value) => Ok(Outcome {
            result: Some(value),
            failure: None,
        }),
        Err(FileWriteError::Provider(error)) => Ok(Outcome {
            result: None,
            failure: Some(Failure {
                status: error.status_code,
                synthesized: error.status_code_is_synthesized,
                message: error.message,
                headers: error.headers,
            }),
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}
