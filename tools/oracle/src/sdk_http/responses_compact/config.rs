use betterlite_context::non_default_params::Params as OptionalParams;
use betterlite_core::api::responses_compact_call::error::CompactParamError;
use betterlite_core::api::responses_compact_http::config::{
    CompactConfig, CompactParams, CompactTransformRequest, Headers, SignRequest, Signed,
};
use betterlite_core::api::responses_compact_http::error::ConfigFailure;
use betterlite_core::api::responses_parameters::provider::ResponsesParamProvider;
use betterlite_core::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ResponsesProvider, ResponsesReply, WorkloadIdentity,
    get_complete_url, transform_response, validate_environment,
};
use betterlite_openai::response_resources::request::{
    CompactRequest as ProviderCompactRequest, ResourceRequest, compact_request,
};
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::api::responses_provider_contract::records::{
    Params as ProviderParams, normalize_responses_request,
};
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub(super) struct OpenAiCompactConfig {
    pub(super) api_key: Option<SecretString>,
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
        let params: ProviderParams = round_trip(&data)?;
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

impl ResponsesParamProvider for OpenAiCompactConfig {
    type Error = CompactParamError;

    fn name(&self) -> &str {
        "openai"
    }

    fn supported_params(&self, _model: &str) -> Result<Option<Vec<String>>, Self::Error> {
        Ok(None)
    }

    fn map_params(
        &self,
        params: &OptionalParams,
        _model: &str,
        _drop_params: bool,
    ) -> Result<OptionalParams, Self::Error> {
        Ok(params.clone())
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
