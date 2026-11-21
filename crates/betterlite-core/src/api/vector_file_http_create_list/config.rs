use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::reply::{ReplyKind, VectorReply, decode_vector_reply};
use betterlite_openai::vector_stores::{
    Headers, complete_url as openai_complete_url,
    validate_environment as openai_validate_environment,
};
use secrecy::SecretString;

use crate::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, UrlFailure, header_map,
};

// The litellm_params fields the four handlers read
#[derive(Debug, Clone, Default)]
pub struct FilesParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
}

// The BaseVectorStoreFilesConfig members these handlers call; B1071's provider trait adapts to this
pub trait FilesConfig: Send + Sync {
    fn validate_environment(&self, headers: Headers, params: &FilesParams) -> Headers;

    fn complete_url(
        &self,
        params: &FilesParams,
        vector_store_id: &str,
    ) -> Result<String, UrlFailure>;

    fn transform_response(&self, reply: &UpstreamResponse) -> Result<Json, BaseLlmException>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}

// OpenAIVectorStoreFilesConfig's members over B527, whose validate_environment rewrites Authorization and Content-Type in the map it is given
pub struct OpenAiFilesConfig<'settings> {
    settings: &'settings OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
    litellm_api_base: Option<String>,
}

impl<'settings> OpenAiFilesConfig<'settings> {
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

impl FilesConfig for OpenAiFilesConfig<'_> {
    fn validate_environment(&self, headers: Headers, params: &FilesParams) -> Headers {
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
        params: &FilesParams,
        vector_store_id: &str,
    ) -> Result<String, UrlFailure> {
        openai_complete_url(
            params.api_base.as_deref(),
            self.litellm_api_base.as_deref(),
            self.settings,
            Some(vector_store_id),
        )
        .map_err(|source| UrlFailure {
            source: Box::new(source),
        })
    }

    fn transform_response(&self, reply: &UpstreamResponse) -> Result<Json, BaseLlmException> {
        let decoded = decode_vector_reply(ReplyKind::File, &reply.body).map_err(|source| {
            self.error_class(
                &source.to_string(),
                reply.status,
                header_map(&reply.headers),
            )
        })?;
        // the file kind decodes as JSON, so a record here means the decoder and this caller disagree
        let VectorReply::Json(value) = decoded else {
            return Err(self.error_class(
                "the file reply is not JSON",
                reply.status,
                header_map(&reply.headers),
            ));
        };
        serde_json::from_value(value).map_err(|source| {
            self.error_class(
                &source.to_string(),
                reply.status,
                header_map(&reply.headers),
            )
        })
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
