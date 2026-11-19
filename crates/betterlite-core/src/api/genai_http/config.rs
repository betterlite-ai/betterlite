use std::future::Future;

use betterlite_http::proxy::UpstreamResponse;
use indexmap::IndexMap;

use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use crate::genai_setup::config::GenerateContentProviderConfig;
use crate::genai_setup::params::GenerateContentLiteLLMParams;

// The litellm_params fields sync_get_auth_token_and_url and get_auth_token_and_url read
pub struct AuthCall<'call> {
    pub api_base: Option<&'call str>,
    pub model: &'call str,
    pub params: &'call GenerateContentLiteLLMParams,
    pub stream: bool,
}

pub struct AuthTarget {
    pub url: String,
    pub headers: IndexMap<String, String>,
}

// The BaseGoogleGenAIGenerateContentConfig members the handler calls beside B3483's request transform; the adapter
// that owns a provider config implements this, since this crate cannot name a betterlite-vertex type.
pub trait GenerateContentHttpConfig: GenerateContentProviderConfig + Send + Sync {
    type Response;

    fn auth_token_and_url(
        &self,
        call: &AuthCall<'_>,
    ) -> impl Future<Output = Result<AuthTarget, Self::Error>> + Send;

    // transform_generate_content_response; a reply it cannot parse becomes the provider's own error class
    fn decode_response(
        &self,
        response: &UpstreamResponse,
    ) -> Result<Self::Response, BaseLlmException>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}
