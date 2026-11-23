use betterlite_context::non_default_params::Params as OptionalParams;
use betterlite_core::api::responses_compact_http::error::ConfigFailure;
use betterlite_core::api::responses_create::deps::CreateCapabilities;
use betterlite_core::api::responses_create::error::CreateParamError;
use betterlite_core::api::responses_create_http::config::{
    CreateConfig, CreateParams, CreateSignRequest, CreateTransformRequest, Headers, Signed,
};
use betterlite_core::api::responses_parameters::provider::ResponsesParamProvider;
use betterlite_core::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ResponsesProvider, ResponsesReply, WorkloadIdentity,
    get_complete_url, transform_response, validate_environment,
};
use betterlite_openai::response_input::transform_responses_request;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::api::responses_provider_contract::records::{
    Params as ContractParams, normalize_responses_request,
};
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
pub(super) struct OpenAiCreateConfig {
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
impl CreateConfig for OpenAiCreateConfig {
    fn validate_environment(
        &self,
        headers: Headers,
        _model: &str,
        params: &CreateParams,
    ) -> Result<Headers, ConfigFailure> {
        let mut identity = NoIdentity;
        Ok(validate_environment(
            headers,
            ApiKeySources {
                litellm_params: params.api_key.as_ref(),
                litellm: None,
                openai: None,
                environment: self.api_key.as_ref(),
            },
            params.api_base.as_deref(),
            ResponsesProvider::OpenAi,
            &mut identity,
        ))
    }
    fn complete_url(&self, params: &CreateParams) -> Result<String, ConfigFailure> {
        Ok(get_complete_url(
            params.api_base.as_deref(),
            None,
            None,
            None,
        ))
    }
    fn transform_request(
        &self,
        request: &CreateTransformRequest<'_>,
    ) -> Result<Object, ConfigFailure> {
        transform_responses_request(
            request.model,
            request.input,
            request.optional_params,
            ProviderId::Openai,
            None,
        )
        .map_err(ConfigFailure::new)
    }
    fn normalize_request(&self, data: Object) -> Result<Object, ConfigFailure> {
        let params: ContractParams = round_trip(&data)?;
        round_trip(&normalize_responses_request(&params))
    }
    fn sign_request(&self, request: &CreateSignRequest<'_>) -> Result<Signed, ConfigFailure> {
        Ok(Signed {
            headers: request.headers.clone(),
            body: None,
        })
    }
    fn transform_response(
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
impl ResponsesParamProvider for OpenAiCreateConfig {
    type Error = CreateParamError;
    fn name(&self) -> &'static str {
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
impl CreateCapabilities for OpenAiCreateConfig {
    fn supports_native_file_search(&self) -> bool {
        true
    }
    fn should_fake_stream(&self, _model: &str, _stream: Option<bool>, _provider: &str) -> bool {
        false
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
