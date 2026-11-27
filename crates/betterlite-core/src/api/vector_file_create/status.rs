use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers;

use crate::api::vector_file_http_create_list::config::{FilesConfig, FilesParams};
use crate::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, UrlFailure, handle_error, status_facts,
};

// LiteLLM's httpx client calls raise_for_status before either transform, so a list reply with an error status
pub struct StatusCheckedFilesConfig<Config: FilesConfig> {
    pub inner: Config,
}

impl<Config: FilesConfig> FilesConfig for StatusCheckedFilesConfig<Config> {
    fn validate_environment(&self, headers: Headers, params: &FilesParams) -> Headers {
        self.inner.validate_environment(headers, params)
    }

    fn complete_url(
        &self,
        params: &FilesParams,
        vector_store_id: &str,
    ) -> Result<String, UrlFailure> {
        self.inner.complete_url(params, vector_store_id)
    }

    fn transform_response(&self, reply: &UpstreamResponse) -> Result<Json, BaseLlmException> {
        if reply.status >= 400 {
            return Err(handle_error(&status_facts(reply), None));
        }
        self.inner.transform_response(reply)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        self.inner.error_class(message, status_code, headers)
    }
}
