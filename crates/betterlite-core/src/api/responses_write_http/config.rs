use std::collections::BTreeMap;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ResponsesProvider, WorkloadIdentity, get_complete_url,
    validate_environment,
};
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::{
    ResourceOperation, ResourceRequest, resource_request,
};
use betterlite_openai::response_resources::response::{TypedReply, decode_record, response_reply};
use secrecy::SecretString;

use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap, UrlFailure};

// The litellm_params fields the four handlers read
#[derive(Debug, Clone, Default)]
pub struct WriteParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
}

// The BaseResponsesAPIConfig members these handlers call; B1114 adapts a provider config to this
pub trait ResponsesConfig: Send + Sync {
    fn validate_environment(
        &self,
        headers: BTreeMap<String, String>,
        params: &WriteParams,
    ) -> BTreeMap<String, String>;

    fn complete_url(&self, params: &WriteParams) -> Result<String, UrlFailure>;

    fn delete_request(
        &self,
        response_id: &str,
        api_base: &str,
    ) -> Result<ResourceRequest, ResponseResourceError>;

    fn cancel_request(
        &self,
        response_id: &str,
        api_base: &str,
    ) -> Result<ResourceRequest, ResponseResourceError>;

    fn delete_response(
        &self,
        reply: &RawResponse<'_>,
    ) -> Result<DeleteResponseResult, ResponseResourceError>;

    fn cancel_response(&self, reply: &RawResponse<'_>)
    -> Result<TypedReply, ResponseResourceError>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}

// OpenAIResponsesAPIConfig's members over B508 and B509, whose validate_environment rewrites Authorization in the map it is given
pub struct OpenAiResponsesConfig<'settings> {
    settings: &'settings OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
    litellm_api_base: Option<String>,
}

impl<'settings> OpenAiResponsesConfig<'settings> {
    #[must_use]
    pub const fn new(
        settings: &'settings OpenAiCredentialsSettings,
        globals: LiteLlmGlobals,
        env_api_key: Option<SecretString>,
        litellm_api_base: Option<String>,
    ) -> Self {
        Self {
            settings,
            globals,
            env_api_key,
            litellm_api_base,
        }
    }
}

// No workload identity is configured on this path, so B508 falls back to the API key
struct NoWorkloadIdentity;

impl WorkloadIdentity for NoWorkloadIdentity {
    fn bearer_token(
        &mut self,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Option<SecretString> {
        None
    }
}

impl ResponsesConfig for OpenAiResponsesConfig<'_> {
    fn validate_environment(
        &self,
        headers: BTreeMap<String, String>,
        params: &WriteParams,
    ) -> BTreeMap<String, String> {
        let sources = ApiKeySources {
            litellm_params: params.api_key.as_ref(),
            litellm: self.globals.api_key.as_ref(),
            openai: self.globals.openai_key.as_ref(),
            environment: self.env_api_key.as_ref(),
        };
        validate_environment(
            headers,
            sources,
            params.api_base.as_deref(),
            ResponsesProvider::OpenAi,
            &mut NoWorkloadIdentity,
        )
    }

    fn complete_url(&self, params: &WriteParams) -> Result<String, UrlFailure> {
        Ok(get_complete_url(
            params.api_base.as_deref(),
            self.litellm_api_base.as_deref(),
            Some(self.settings.api_base.as_str()),
            None,
        ))
    }

    fn delete_request(
        &self,
        response_id: &str,
        api_base: &str,
    ) -> Result<ResourceRequest, ResponseResourceError> {
        resource_request(response_id, api_base, ResourceOperation::Delete)
    }

    fn cancel_request(
        &self,
        response_id: &str,
        api_base: &str,
    ) -> Result<ResourceRequest, ResponseResourceError> {
        resource_request(response_id, api_base, ResourceOperation::Cancel)
    }

    fn delete_response(
        &self,
        reply: &RawResponse<'_>,
    ) -> Result<DeleteResponseResult, ResponseResourceError> {
        decode_record(reply)
    }

    fn cancel_response(
        &self,
        reply: &RawResponse<'_>,
    ) -> Result<TypedReply, ResponseResourceError> {
        response_reply(reply)
    }

    // BaseResponsesAPIConfig.get_error_class raises this exception, so asking it and building one are the same answer here
    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}
