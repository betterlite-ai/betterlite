use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::file_http_metadata::config::FileParams;
use betterlite_core::api::file_http_metadata::error::MetadataError;
use betterlite_core::api::file_http_metadata::service::{
    DeleteCall, ListCall, MetadataCall, MetadataDeps, MetadataTarget, MetadataValue, RetrieveCall,
};
use serde_json::Value;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, source};
use super::xai::XaiFilesConfig;
use super::{Call, Failure, Operation, Outcome};

// The production service over the mock's base: the same client pool and the xAI file contract a request would pick
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let config = loader
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let origin = source(&config, env)?;
    let provider = XaiFilesConfig {
        settings: &config.sections.xai_identity,
    };
    let mut fixture = Fixture::new(call.logging_call_type())?;
    let mut params = FileParams::new();
    params.insert("api_base".to_owned(), Value::String(api_base.to_owned()));
    if let Some(key) = &call.api_key {
        params.insert("api_key".to_owned(), Value::String(key.clone()));
    }
    let metadata = metadata_call(call, &params)?;
    let deps = MetadataDeps {
        config: &provider,
        source: &origin,
        supplied: None,
    };
    let value = fixture
        .execute(&deps, call.mode.client_kind(), &metadata)
        .await;
    Ok(SdkResult::FileHttpMetadata {
        value: Box::new(outcome(value)?),
    })
}

// The target is built once, and the operation's id or purpose is what its variant carries
fn metadata_call<'call>(
    call: &'call Call,
    params: &'call FileParams,
) -> Result<MetadataCall<'call>, SdkHttpError> {
    let target = MetadataTarget {
        litellm_params: params,
        extra_headers: call.extra_headers.as_ref(),
        ssl_verify: None,
        timeout: call.timeout_seconds.map(Duration::from_secs),
    };
    let missing =
        || SdkHttpError::Invalid(String::from("a retrieve or delete call needs a file_id"));
    match call.operation {
        Operation::Retrieve => Ok(MetadataCall::Retrieve(RetrieveCall {
            file_id: call.file_id.as_deref().ok_or_else(missing)?,
            target,
        })),
        Operation::Delete => Ok(MetadataCall::Delete(DeleteCall {
            file_id: call.file_id.as_deref().ok_or_else(missing)?,
            target,
        })),
        Operation::List => Ok(MetadataCall::List(ListCall {
            purpose: call.purpose.as_deref(),
            target,
        })),
    }
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(result: Result<MetadataValue, MetadataError>) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(value) => {
            let text = match value {
                MetadataValue::File(file) => serde_json::to_string(&file),
                MetadataValue::Deleted(deleted) => serde_json::to_string(&deleted),
                MetadataValue::Listed(files) => serde_json::to_string(&files),
            }
            .map_err(SdkHttpError::Serialize)?;
            let json = serde_json::from_str(&text).map_err(SdkHttpError::Serialize)?;
            Ok(Outcome {
                result: Some(json),
                failure: None,
            })
        }
        Err(MetadataError::Provider(error)) => Ok(Outcome {
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
