use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::responses_write_http::config::{OpenAiResponsesConfig, WriteParams};
use betterlite_core::api::responses_write_http::error::WriteError;
use betterlite_core::api::responses_write_http::service::{
    CancelCall, DeleteCall, WriteCall, WriteDeps, WriteOutcome, WriteTarget, send,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::response_resources::response::TypedReply;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;

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
    // The OpenAI responses config over the env's key, exactly as the proxy wires B527
    let provider = OpenAiResponsesConfig::new(
        &config.sections.openai_credentials,
        LiteLlmGlobals::default(),
        config.sections.resource_sdk_settings.openai_api_key.clone(),
        None,
    );
    let origin = source(&config, env)?;
    let mut fixture = Fixture::new("responses")?;
    let params = WriteParams {
        api_base: Some(api_base.to_owned()),
        api_key: call.api_key.clone().map(SecretString::from),
        ssl_verify: None,
    };
    let write = write_call(call, &params);
    let deps = WriteDeps {
        config: &provider,
        source: &origin,
        supplied: None,
        shared_session: None,
    };
    let prepared = fixture.prepare(&deps, call.mode.client_kind(), &write)?;
    let answer = outcome(send(prepared).await)?;
    Ok(SdkResult::ResponsesWriteTransport {
        value: Box::new(answer),
    })
}

fn write_call<'call>(call: &'call Call, params: &'call WriteParams) -> WriteCall<'call> {
    let target = WriteTarget {
        response_id: &call.response_id,
        params,
        extra_headers: call.extra_headers.as_ref(),
        timeout: call.timeout_seconds.map(Duration::from_secs),
    };
    match call.operation {
        Operation::Delete => WriteCall::Delete(DeleteCall {
            target,
            extra_body: call.extra_body.as_ref(),
        }),
        Operation::Cancel => WriteCall::Cancel(CancelCall {
            target,
            extra_body: call.extra_body.as_ref(),
        }),
    }
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(result: Result<WriteOutcome, WriteError>) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(WriteOutcome::Deleted(record)) => Ok(Outcome {
            result: Some(deleted_value(&record)),
            failure: None,
        }),
        Ok(WriteOutcome::Cancelled(typed)) => Ok(Outcome {
            result: Some(cancelled_value(&typed)),
            failure: None,
        }),
        Err(WriteError::Provider(error)) => Ok(Outcome {
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

fn deleted_value(record: &DeleteResponseResult) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        String::from("id"),
        record.id.clone().map_or(Json::Null, Json::Text),
    );
    fields.insert(
        String::from("object"),
        record.object.clone().map_or(Json::Null, Json::Text),
    );
    fields.insert(
        String::from("deleted"),
        record.deleted.map_or(Json::Null, Json::Bool),
    );
    Json::Object(fields)
}

fn cancelled_value(typed: &TypedReply) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(String::from("id"), Json::Text(typed.response.id.clone()));
    fields.insert(
        String::from("created_at"),
        Json::Number(Number::from(typed.response.created_at)),
    );
    fields.insert(
        String::from("object"),
        typed.response.object.clone().map_or(Json::Null, Json::Text),
    );
    fields.insert(
        String::from("status"),
        typed.response.status.clone().map_or(Json::Null, Json::Text),
    );
    fields.insert(
        String::from("model"),
        typed.response.model.clone().map_or(Json::Null, Json::Text),
    );
    let headers: IndexMap<String, Json> = typed
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    fields.insert(String::from("headers"), Json::Object(headers));
    fields.insert(
        String::from("additional_headers"),
        Json::Object(typed.additional_headers.clone()),
    );
    Json::Object(fields)
}
