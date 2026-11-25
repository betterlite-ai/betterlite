use betterlite_context::non_default_params::Params as OptionalParams;
use betterlite_core::api::response_id_traversal::response_id::transcode;
use betterlite_core::api::responses_compact_http::error::ConfigFailure;
use betterlite_core::api::responses_create::deps::CreateCapabilities;
use betterlite_core::api::responses_create_http::config::{
    CreateConfig, CreateParams, CreateSignRequest, CreateTransformRequest, Headers, Signed,
};
use betterlite_core::api::responses_parameters::error::ResponsesParametersError;
use betterlite_core::api::responses_parameters::provider::ResponsesParamProvider;
use betterlite_core::api::responses_read_http::config::NoWorkloadIdentity;
use betterlite_core::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ResponsesProvider, ResponsesReply, get_complete_url,
    transform_response, validate_environment,
};
use betterlite_openai::response_input::transform_responses_request;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::api::responses_provider_contract::records::{
    Params as ContractParams, normalize_responses_request,
};
use secrecy::SecretString;
pub(super) struct OpenAiCreateConfig {
    pub(super) api_key: Option<SecretString>,
}
impl CreateConfig for OpenAiCreateConfig {
    fn validate_environment(
        &self,
        headers: Headers,
        _model: &str,
        params: &CreateParams,
    ) -> Result<Headers, ConfigFailure> {
        let mut identity = NoWorkloadIdentity;
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
        let params: ContractParams = transcode(&data).map_err(ConfigFailure::new)?;
        transcode(&normalize_responses_request(&params)).map_err(ConfigFailure::new)
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
    type Error = ResponsesParametersError;
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
