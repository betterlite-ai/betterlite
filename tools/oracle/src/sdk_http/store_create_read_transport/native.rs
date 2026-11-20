use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_store_http_create_read::config::OpenAiCreateReadConfig;
use betterlite_core::api::vector_store_http_create_read::error::CreateReadError;
use betterlite_core::api::vector_store_http_create_read::service::{
    CreateCall, CreateReadCall, CreateReadDeps, CreateReadTarget, ListCall, ListQuery,
    RetrieveCall, send,
};
use betterlite_core::api::vector_store_http_write::config::WriteParams;
use betterlite_core::api::vector_store_http_write::error::WriteError;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::store::CreateParams;

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
    let provider = OpenAiCreateReadConfig::new(
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
    // The five optional fields as the production CreateParams, through the JSON their names describe
    let encoded = serde_json::to_value(&call.optional_params).map_err(SdkHttpError::Serialize)?;
    let optional: CreateParams = serde_json::from_value(encoded)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let read = read_call(call, &params, &optional)?;
    let deps = CreateReadDeps {
        config: &provider,
        source: &origin,
        supplied: None,
    };
    let prepared = fixture.prepare(&deps, call.mode.client_kind(), &read)?;
    let answer = outcome(send(prepared).await)?;
    Ok(SdkResult::VectorStoreCreateReadTransport {
        value: Box::new(answer),
    })
}

fn read_call<'call>(
    call: &'call Call,
    params: &'call WriteParams,
    optional: &'call CreateParams,
) -> Result<CreateReadCall<'call>, SdkHttpError> {
    let target = CreateReadTarget {
        params,
        extra_headers: call.extra_headers.as_ref(),
        timeout: call.timeout_seconds.map(Duration::from_secs),
    };
    let read = match call.operation {
        Operation::Create => CreateReadCall::Create(CreateCall {
            target,
            optional_params: optional,
            extra_body: call.extra_body.as_ref(),
        }),
        Operation::Retrieve => {
            let vector_store_id = call.vector_store_id.as_deref().ok_or_else(|| {
                SdkHttpError::Invalid("a retrieve call needs a vector store id".to_owned())
            })?;
            CreateReadCall::Retrieve(RetrieveCall {
                target,
                vector_store_id,
                extra_body: call.extra_body.as_ref(),
            })
        }
        Operation::List => CreateReadCall::List(ListCall {
            target,
            query: ListQuery {
                after: call.after.as_deref(),
                before: call.before.as_deref(),
                limit: call.limit,
                order: call.order.as_deref(),
            },
            extra_body: call.extra_body.as_ref(),
        }),
    };
    Ok(read)
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(result: Result<Json, CreateReadError>) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(value) => Ok(Outcome {
            result: Some(value),
            failure: None,
        }),
        Err(CreateReadError::Write(WriteError::Provider(error))) => Ok(Outcome {
            result: None,
            failure: Some(Failure {
                status: error.status_code,
                synthesized: error.status_code_is_synthesized,
                message: error.message,
                headers: error.headers,
            }),
        }),
        Err(CreateReadError::Write(other)) => Err(SdkHttpError::Invalid(other.to_string())),
        Err(CreateReadError::Capability(error)) => Err(SdkHttpError::Invalid(error.to_string())),
    }
}
