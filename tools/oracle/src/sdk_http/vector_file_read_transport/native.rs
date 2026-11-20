use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_file_http_read::config::{OpenAiReadConfig, ReadParams};
use betterlite_core::api::vector_file_http_read::error::ReadError;
use betterlite_core::api::vector_file_http_read::service::{ReadDeps, ReadTarget, send};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, source};
use super::{Call, Failure, Outcome};

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
        let mut fixture = Fixture::new(operation.logging_call_type())?;
        let target = ReadTarget {
            vector_store_id: &call.vector_store_id,
            file_id: &call.file_id,
            params: &params,
            extra_headers: call.extra_headers.as_ref(),
            timeout: call.timeout_seconds.map(Duration::from_secs),
        };
        let prepared = fixture.prepare(
            &deps,
            call.mode.client_kind(),
            operation.read_operation(),
            &target,
        )?;
        match send(prepared).await {
            Ok(value) => results.push(value),
            Err(ReadError::Provider(error)) => {
                return Ok(SdkResult::VectorFileReadTransport {
                    value: Box::new(Outcome {
                        results: Vec::new(),
                        failure: Some(Failure {
                            status: error.status_code,
                            synthesized: error.status_code_is_synthesized,
                            message: error.message,
                            headers: error.headers,
                        }),
                    }),
                });
            }
            Err(other) => return Err(SdkHttpError::Invalid(other.to_string())),
        }
    }
    Ok(SdkResult::VectorFileReadTransport {
        value: Box::new(Outcome {
            results,
            failure: None,
        }),
    })
}
