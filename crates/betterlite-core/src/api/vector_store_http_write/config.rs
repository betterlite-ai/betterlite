use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::reply::{ReplyKind, VectorReply, transform_response};
use betterlite_openai::vector_stores::{
    Headers, complete_url as openai_complete_url,
    validate_environment as openai_validate_environment,
};
use secrecy::SecretString;

use super::error::{BaseLlmException, HeaderMap, UrlFailure, header_map};

// The litellm_params fields the four handlers read
#[derive(Debug, Clone, Default)]
pub struct WriteParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
}

// The BaseVectorStoreConfig members these handlers call; B1066 adapts a provider config to this
pub trait WriteConfig: Send + Sync {
    fn validate_environment(&self, headers: Headers, params: &WriteParams) -> Headers;

    fn complete_url(&self, params: &WriteParams) -> Result<String, UrlFailure>;

    fn transform_create_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}

// OpenAIVectorStoreConfig's members over B527, whose validate_environment rewrites Authorization and Content-Type in the map it is given
pub struct OpenAiWriteConfig<'settings> {
    settings: &'settings OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
    litellm_api_base: Option<String>,
}

impl<'settings> OpenAiWriteConfig<'settings> {
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

impl WriteConfig for OpenAiWriteConfig<'_> {
    fn validate_environment(&self, headers: Headers, params: &WriteParams) -> Headers {
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

    fn complete_url(&self, params: &WriteParams) -> Result<String, UrlFailure> {
        openai_complete_url(
            params.api_base.as_deref(),
            self.litellm_api_base.as_deref(),
            self.settings,
            None,
        )
        .map_err(|source| UrlFailure {
            source: Box::new(source),
        })
    }

    fn transform_create_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        let status = response.status().as_u16();
        let headers = header_map(response.headers());
        let converted = match transform_response(ReplyKind::Create, response) {
            Ok(VectorReply::Record(record)) => serde_json::to_value(&*record),
            Ok(VectorReply::Json(value)) => serde_json::to_value(&value),
            Err(source) => return Err(self.error_class(&source.to_string(), status, headers)),
        };
        let encoded = match converted {
            Ok(encoded) => encoded,
            Err(source) => return Err(self.error_class(&source.to_string(), status, headers)),
        };
        serde_json::from_value(encoded)
            .map_err(|source| self.error_class(&source.to_string(), status, headers))
    }

    // BaseVectorStoreConfig.get_error_class raises this exception, so asking it and building one are the same answer here
    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}
