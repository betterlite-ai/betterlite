use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_file_http_create_list::config::{FilesParams, OpenAiFilesConfig};
use betterlite_core::api::vector_file_http_create_list::error::FileTransportError;
use betterlite_core::api::vector_file_http_create_list::service::{
    CreateCall, FileCall, FileDeps, FileTarget, ListCall, send,
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
    let provider = OpenAiFilesConfig::new(
        &config.sections.openai_credentials,
        LiteLlmGlobals::default(),
        config.sections.resource_sdk_settings.openai_api_key.clone(),
        None,
    );
    let origin = source(&config, env)?;
    let mut fixture = Fixture::new(call.logging_call_type())?;
    let files = FilesParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        ssl_verify: None,
    };
    let transport_call = file_call(call, &files);
    let deps = FileDeps {
        config: &provider,
        source: &origin,
        supplied: None,
    };
    let prepared = fixture.prepare(&deps, call.mode.client_kind(), &transport_call)?;
    let answer = outcome(send(prepared).await)?;
    Ok(SdkResult::VectorFileCreateListTransport {
        value: Box::new(answer),
    })
}

fn file_call<'call>(call: &'call Call, files: &'call FilesParams) -> FileCall<'call> {
    let target = FileTarget {
        vector_store_id: &call.vector_store_id,
        params: files,
        extra_headers: call.extra_headers.as_ref(),
        timeout: call.timeout_seconds.map(Duration::from_secs),
    };
    match call.operation {
        Operation::Create => FileCall::Create(CreateCall {
            target,
            create_request: &call.create_request,
            extra_body: call.extra_body.as_ref(),
        }),
        Operation::List => FileCall::List(ListCall {
            target,
            query_params: &call.query_params,
            extra_query: call.extra_query.as_ref(),
        }),
    }
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(result: Result<Json, FileTransportError>) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(value) => Ok(Outcome {
            result: Some(value),
            failure: None,
        }),
        Err(FileTransportError::Provider(error)) => Ok(Outcome {
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
