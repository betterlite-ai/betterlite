use std::time::{Duration, SystemTime};

use aws_credential_types::provider::error::CredentialsError;
use betterlite_bedrock::aws_settings::sources::{Credentials, SourceError};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::mantle_settings::auth::{SignRequest, sign_request};
use betterlite_bedrock::mantle_settings::error::{AUTH_FAILURE_MESSAGE, MantleSettingsError};
use betterlite_bedrock::mantle_settings::messages::{
    CUSTOM_LLM_PROVIDER, USES_GET_LLM_PROVIDER_API_BASE, build_native_messages_url,
};
use betterlite_bedrock::mantle_settings::passthrough::{
    PassthroughEndpointRequest, logging_non_streaming_response, passthrough_bearer_token,
    passthrough_region, passthrough_runtime_endpoint,
};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{PassthroughTarget, RelayedResponse};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::fixtures;
use super::input::{FailureKind, Input};
use super::ops::{globals, object_with_region};
use betterlite_oracle_helper_api::helper::HelperResult;

fn failed_credentials(kind: FailureKind) -> Result<Credentials, CredentialError> {
    match kind {
        FailureKind::NoCredentials | FailureKind::Region => {
            Err(CredentialError::MissingCredentials)
        }
        FailureKind::PartialCredentials
        | FailureKind::ProfileNotFound
        | FailureKind::CredentialRetrieval => Err(CredentialError::Source(SourceError::Provider(
            CredentialsError::provider_error("fixture credential failure"),
        ))),
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Messages { cases } => {
            let mut urls = Vec::with_capacity(cases.len());
            for case in cases {
                let globals = globals(&case.environment);
                let params = object_with_region(case.aws_region_name.as_deref());
                let url = build_native_messages_url(case.api_base.as_deref(), &params, &globals);
                urls.push(
                    url.map_or_else(|_error| json!({"error": true}), |url| json!({"url": url})),
                );
            }
            Ok(json!({
                "provider": <&'static str>::from(CUSTOM_LLM_PROVIDER),
                "uses_get_llm_provider_api_base": USES_GET_LLM_PROVIDER_API_BASE,
                "urls": urls,
            }))
        }
        Input::Passthrough { cases } => {
            let mut results = Vec::with_capacity(cases.len());
            for case in cases {
                let globals = globals(&case.environment);
                let mut params = object_with_region(case.aws_region_name.as_deref());
                if let Some(base) = &case.api_base {
                    params.insert("api_base".to_owned(), json!(base));
                }
                let Ok(region) = passthrough_region(&params, &globals) else {
                    results.push(json!({"error": true}));
                    continue;
                };
                let endpoint = passthrough_runtime_endpoint(PassthroughEndpointRequest {
                    api_base: case.api_base.as_deref(),
                    configured_endpoint: case.aws_bedrock_runtime_endpoint.as_deref(),
                    environment_endpoint: case
                        .environment
                        .get("AWS_BEDROCK_RUNTIME_ENDPOINT")
                        .map(String::as_str),
                    region: &region,
                    endpoint_type: case.endpoint_type,
                });
                let mut litellm_params = Object::new();
                if let Some(key) = &case.api_key {
                    litellm_params.insert("api_key".to_owned(), key.clone());
                }
                let bearer = passthrough_bearer_token(&litellm_params, &globals)
                    .map(|token| token.expose_secret().to_owned());
                results.push(json!({
                    "region": region,
                    "endpoint": [endpoint.endpoint_url, endpoint.proxy_endpoint_url],
                    "bearer": bearer,
                }));
            }
            Ok(json!({"results": results}))
        }
        Input::Failure { cases } => {
            let runtime = Builder::new_current_thread().build()?;
            let mut results = Vec::with_capacity(cases.len());
            for case in cases {
                let globals = globals(&case.environment);
                let params = object_with_region(case.aws_region_name.as_deref());
                let data = IndexMap::from([(
                    String::from("messages"),
                    Json::List(vec![Json::Object(IndexMap::new())]),
                )]);
                let api_base = case
                    .api_base
                    .as_deref()
                    .unwrap_or("https://bedrock-mantle.us-east-1.api.aws/v1");
                let kind = case.kind;
                let request = SignRequest {
                    headers: &[],
                    optional_params: &params,
                    request_data: &data,
                    api_base,
                    api_key: None,
                    model: Some("openai.gpt-oss-120b"),
                    stream: None,
                    fake_stream: None,
                    globals: &globals,
                    now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
                };
                let outcome = runtime.block_on(sign_request(
                    request,
                    move |_settings, _region| async move { failed_credentials(kind) },
                ));
                results.push(match outcome {
                    Ok(_signed) => json!({"kind": "signed"}),
                    Err(MantleSettingsError::Auth { .. }) => {
                        json!({"kind": "auth_failure", "message": AUTH_FAILURE_MESSAGE})
                    }
                    Err(MantleSettingsError::Region(_)) => json!({"kind": "region_error"}),
                    Err(_error) => json!({"kind": "other"}),
                });
            }
            Ok(json!({"results": results}))
        }
        Input::ReplyShape { cases } => {
            let mut results = Vec::with_capacity(cases.len());
            for case in cases {
                let target = PassthroughTarget {
                    model: "fixture-model",
                    custom_llm_provider: case.custom_llm_provider.as_str(),
                    endpoint: case.endpoint.as_str(),
                };
                let response = RelayedResponse {
                    status_code: 200,
                    body: b"{}",
                };
                let mut request_data = Object::new();
                request_data.insert("stream".to_owned(), json!(false));
                let forwarded = logging_non_streaming_response(
                    target,
                    &response,
                    &request_data,
                    |seen, _, data| {
                        Ok::<
                            Option<Value>,
                            betterlite_bedrock::passthrough::error::PassthroughFailure,
                        >(Some(json!({
                            "model": seen.model,
                            "provider": seen.custom_llm_provider,
                            "endpoint": seen.endpoint,
                            "request_data": data,
                        })))
                    },
                )
                .map_err(fixtures::failure)?;
                results.push(forwarded.unwrap_or(Value::Null));
            }
            Ok(json!({"results": results}))
        }
        _ => Err(fixtures::failure(
            "mantle outcomes do not handle this operation",
        ))?,
    }
}
