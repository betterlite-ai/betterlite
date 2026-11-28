use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_file_http_read::config::{
    OpenAiReadConfig, ReadConfig, ReadParams,
};
use betterlite_core::api::vector_file_http_read::error::ReadError;
use betterlite_core::api::vector_file_http_read::service::{
    ReadDeps, ReadTarget, ReadTransport, send,
};
use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, source};
use super::{Call, Failure, Operation, Outcome};

// The production handlers over the mock's base: the same config sections and pooled client a request would build
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
    let provider = OpenAiReadConfig::new(
        &config.sections.openai_credentials,
        LiteLlmGlobals::default(),
        config.sections.resource_sdk_settings.openai_api_key.clone(),
        None,
    );
    let origin = source(&config, env)?;
    let params = ReadParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        ssl_verify: None,
    };
    let deps = ReadDeps {
        config: &provider,
        source: &origin,
        supplied: None,
    };
    let mut results: Vec<Json> = Vec::new();
    for operation in &call.operations {
        match read_one(&deps, call, *operation, &params).await? {
            ReadAnswer::Value(value) => results.push(value),
            ReadAnswer::Failed(failure) => return Ok(transport_result(Vec::new(), Some(failure))),
        }
    }
    Ok(transport_result(results, None))
}

// What one read produced: a parsed reply, or the provider error that stops the call
enum ReadAnswer {
    Value(Json),
    Failed(Failure),
}

// One operation's send through the handler's front half, with the provider error kept as the case's failure
async fn read_one<'borrow, 'deps, C, S>(
    deps: &'borrow ReadDeps<'deps, C, S>,
    call: &Call,
    operation: Operation,
    params: &ReadParams,
) -> Result<ReadAnswer, SdkHttpError>
where
    'deps: 'borrow,
    C: ReadConfig,
    S: ClientSource<Client: ReadTransport>,
{
    let mut fixture = Fixture::new(operation.logging_call_type())?;
    let target = ReadTarget {
        vector_store_id: &call.vector_store_id,
        file_id: &call.file_id,
        params,
        extra_headers: call.extra_headers.as_ref(),
        timeout: call.timeout_seconds.map(Duration::from_secs),
    };
    let prepared = fixture.prepare(
        deps,
        call.mode.client_kind(),
        operation.read_operation(),
        &target,
    )?;
    match send(prepared).await {
        Ok(value) => Ok(ReadAnswer::Value(value)),
        Err(ReadError::Provider(error)) => Ok(ReadAnswer::Failed(Failure {
            status: error.status_code,
            synthesized: error.status_code_is_synthesized,
            message: error.message,
            headers: error.headers,
        })),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

fn transport_result(results: Vec<Json>, failure: Option<Failure>) -> SdkResult {
    SdkResult::VectorFileReadTransport {
        value: Box::new(Outcome { results, failure }),
    }
}
