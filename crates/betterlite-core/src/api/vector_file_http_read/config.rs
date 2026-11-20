use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::files::{FileOperation, file_request};
use betterlite_openai::vector_stores::reply::{ReplyKind, transform_response};
use betterlite_openai::vector_stores::{
    Headers, complete_url as openai_complete_url,
    validate_environment as openai_validate_environment,
};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap, header_map};

use super::error::PrepareFailure;

// LiteLLM's request transform returns dict[str, str]; its order is the order a query string appends in
pub type QueryParams = IndexMap<String, String>;

// The litellm_params fields the four handlers read
#[derive(Debug, Clone, Default)]
pub struct ReadParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
}

// The BaseVectorStoreFilesConfig members these handlers call; B527 backs the OpenAI one
pub trait ReadConfig: Send + Sync {
    fn validate_environment(&self, headers: Headers, params: &ReadParams) -> Headers;

    fn complete_url(
        &self,
        params: &ReadParams,
        vector_store_id: &str,
    ) -> Result<String, PrepareFailure>;

    fn transform_retrieve_request(
        &self,
        file_id: &str,
        api_base: &str,
    ) -> Result<(String, QueryParams), PrepareFailure>;

    fn transform_content_request(
        &self,
        file_id: &str,
        api_base: &str,
    ) -> Result<(String, QueryParams), PrepareFailure>;

    fn transform_retrieve_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException>;

    fn transform_content_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}

// OpenAIVectorStoreFilesConfig's members over B527, whose validate_environment rewrites Authorization and Content-Type in the map it is given
pub struct OpenAiReadConfig<'settings> {
    settings: &'settings OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
    litellm_api_base: Option<String>,
}

impl<'settings> OpenAiReadConfig<'settings> {
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

impl ReadConfig for OpenAiReadConfig<'_> {
    fn validate_environment(&self, headers: Headers, params: &ReadParams) -> Headers {
        openai_validate_environment(
            headers,
            params.api_key.clone(),
            LiteLlmGlobals {
                api_base: self.globals.api_base.clone(),
                api_key: self.globals.api_key.clone(),
                openai_key: self.globals.openai_key.clone(),
                organization: self.globals.organization.clone(),
            },
            self.env_api_key.clone(),
        )
    }

    fn complete_url(
        &self,
        params: &ReadParams,
        vector_store_id: &str,
    ) -> Result<String, PrepareFailure> {
        openai_complete_url(
            params.api_base.as_deref(),
            self.litellm_api_base.as_deref(),
            self.settings,
            Some(vector_store_id),
        )
        .map_err(|source| PrepareFailure {
            source: Box::new(source),
        })
    }

    fn transform_retrieve_request(
        &self,
        file_id: &str,
        api_base: &str,
    ) -> Result<(String, QueryParams), PrepareFailure> {
        request_parts(&FileOperation::Retrieve, file_id, api_base)
    }

    fn transform_content_request(
        &self,
        file_id: &str,
        api_base: &str,
    ) -> Result<(String, QueryParams), PrepareFailure> {
        request_parts(&FileOperation::Content, file_id, api_base)
    }

    fn transform_retrieve_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        file_reply(self, response)
    }

    fn transform_content_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        file_reply(self, response)
    }

    // BaseVectorStoreFilesConfig.get_error_class raises this exception, so asking it and building one are the same answer here
    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

// B527 appends the encoded file segment and the optional content suffix and returns an empty parameter map for both reads
fn request_parts(
    operation: &FileOperation,
    file_id: &str,
    api_base: &str,
) -> Result<(String, QueryParams), PrepareFailure> {
    let request = file_request(operation, file_id, api_base).map_err(|source| PrepareFailure {
        source: Box::new(source),
    })?;
    let params = serde_json::from_value(request.body).map_err(|source| PrepareFailure {
        source: Box::new(source),
    })?;
    Ok((request.url, params))
}

// B527's File reply transform keeps the parsed JSON, so an array or a scalar stays what the provider sent
fn file_reply(
    config: &OpenAiReadConfig<'_>,
    response: &http::Response<Vec<u8>>,
) -> Result<Json, BaseLlmException> {
    let status = response.status().as_u16();
    let headers = header_map(response.headers());
    let reply = match transform_response(ReplyKind::File, response) {
        Ok(reply) => serde_json::to_value(&reply),
        Err(source) => return Err(config.error_class(&source.to_string(), status, headers)),
    };
    let encoded = match reply {
        Ok(encoded) => encoded,
        Err(source) => return Err(config.error_class(&source.to_string(), status, headers)),
    };
    serde_json::from_value(encoded)
        .map_err(|source| config.error_class(&source.to_string(), status, headers))
}
