use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::responses_compact_http::config::{
    CompactConfig, CompactParams, CompactTransformRequest, Headers, SignRequest, Signed,
};
use betterlite_core::api::responses_compact_http::error::{CompactError, ConfigFailure};
use betterlite_core::api::responses_compact_http::service::{CompactCall, CompactDeps};
use betterlite_core::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ReplyBody, ResponsesProvider, ResponsesReply, WorkloadIdentity,
    get_complete_url, transform_response, validate_environment,
};
use betterlite_openai::response_resources::request::{
    CompactRequest as ProviderCompactRequest, ResourceRequest, compact_request,
};
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::api::responses_provider_contract::records::{
    Params, normalize_responses_request,
};
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, source};
use super::{Call, Failure, Outcome};

// The OpenAI responses config over the case env's key, as the compact handler receives it
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let loaded = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let origin = source(&loaded, env)?;
    let config = OpenAiCompactConfig {
        api_key: env.get("OPENAI_API_KEY").cloned().map(SecretString::from),
    };
    let mut fixture = Fixture::new("compact_response")?;
    let params = compact_params(api_base, call.api_query.as_deref());
    let deps = CompactDeps {
        config: &config,
        source: &origin,
        supplied: None,
    };
    let answer = fixture
        .compact(&deps, call.mode.client_kind(), &compact_call(call, &params))
        .await;
    Ok(SdkResult::ResponsesCompactTransport {
        value: Box::new(outcome(answer)?),
    })
}

fn compact_params(api_base: &str, query: Option<&str>) -> CompactParams {
    CompactParams {
        api_base: Some(base_with_query(api_base, query)),
        api_key: None,
        ssl_verify: None,
    }
}

fn base_with_query(api_base: &str, query: Option<&str>) -> String {
    query.map_or_else(
        || api_base.to_owned(),
        |suffix| format!("{api_base}?{suffix}"),
    )
}

fn compact_call<'call>(call: &'call Call, params: &'call CompactParams) -> CompactCall<'call> {
    CompactCall {
        model: &call.model,
        input: &call.input,
        optional_params: &call.optional_params,
        params,
        extra_headers: call.extra_headers.as_ref(),
        extra_body: call.extra_body.as_ref(),
        timeout: call.timeout_seconds.map(Duration::from_secs),
        shared_session: None,
    }
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(result: Result<ResponsesReply, CompactError>) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(reply) => Ok(Outcome {
            result: Some(reply_json(reply)?),
            failure: None,
        }),
        Err(CompactError::Provider(error)) => Ok(Outcome {
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

// The reply as LiteLLM's ResponsesAPIResponse dumps it: a validated reply as the typed model, a lax one as sent
fn reply_json(reply: ResponsesReply) -> Result<Json, SdkHttpError> {
    match reply.response {
        ReplyBody::Typed(response) => {
            let value = serde_json::to_value(*response).map_err(SdkHttpError::Serialize)?;
            serde_json::from_value(value).map_err(|error| SdkHttpError::Invalid(error.to_string()))
        }
        ReplyBody::Lax(fields) => Ok(Json::Object(fields)),
    }
}

struct OpenAiCompactConfig {
    api_key: Option<SecretString>,
}

struct NoIdentity;

impl WorkloadIdentity for NoIdentity {
    fn bearer_token(
        &mut self,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Option<SecretString> {
        None
    }
}

impl CompactConfig for OpenAiCompactConfig {
    fn validate_environment(
        &self,
        headers: Headers,
        _model: &str,
        _params: &CompactParams,
    ) -> Result<Headers, ConfigFailure> {
        let mut identity = NoIdentity;
        Ok(validate_environment(
            headers,
            ApiKeySources {
                litellm_params: None,
                litellm: None,
                openai: None,
                environment: self.api_key.as_ref(),
            },
            None,
            ResponsesProvider::OpenAi,
            &mut identity,
        ))
    }

    fn complete_url(&self, params: &CompactParams) -> Result<String, ConfigFailure> {
        Ok(get_complete_url(
            params.api_base.as_deref(),
            None,
            None,
            None,
        ))
    }

    fn transform_compact_request(
        &self,
        request: &CompactTransformRequest<'_>,
    ) -> Result<ResourceRequest, ConfigFailure> {
        let provider_request = ProviderCompactRequest {
            model: request.model,
            input: request.input,
            options: request.optional_params,
            provider: ProviderId::Openai,
            model_info: None,
        };
        compact_request(&provider_request, request.api_base).map_err(ConfigFailure::new)
    }

    fn normalize_request(&self, data: Object) -> Result<Object, ConfigFailure> {
        let params: Params = round_trip(&data)?;
        round_trip(&normalize_responses_request(&params))
    }

    fn sign_request(&self, request: &SignRequest<'_>) -> Result<Signed, ConfigFailure> {
        Ok(Signed {
            headers: request.headers.clone(),
            body: None,
        })
    }

    fn transform_compact_response(
        &self,
        reply: &RawResponse<'_>,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ResponsesReply, ConfigFailure> {
        transform_response(reply, env.now.timestamp(), state, env).map_err(ConfigFailure::new)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

// The two response shapes are separate Json types, so the normalization sees the provider contract's one
fn round_trip<Wire, Body>(data: &Body) -> Result<Wire, ConfigFailure>
where
    Wire: DeserializeOwned,
    Body: Serialize + ?Sized,
{
    serde_json::to_value(data)
        .and_then(serde_json::from_value::<Wire>)
        .map_err(ConfigFailure::new)
}
