use crate::json::Object;
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_config::decode::response_format;
use crate::ocr_error::OcrError;
use crate::ocr_http::{CallHooks, OcrClient, OcrWireRequest};
use crate::ocr_provider::OcrConfigKind;
use crate::response::OCRResponse;
use async_trait::async_trait;
use secrecy::SecretString;

pub struct OcrCallContext {
    pub model: String,
    pub custom_llm_provider: &'static str,
    pub optional_params: Object,
    pub secret_fields: Vec<String>,
    pub api_key: Option<SecretString>,
}

#[async_trait]
pub trait OcrInterceptors: Send + Sync {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
        context: &OcrCallContext,
    ) -> Result<OcrWireRequest, OcrError>;

    async fn response_received(&self, body: &[u8]) -> Result<(), OcrError>;
}

#[async_trait]
impl OcrInterceptors for () {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
        _context: &OcrCallContext,
    ) -> Result<OcrWireRequest, OcrError> {
        Ok(request)
    }

    async fn response_received(&self, _body: &[u8]) -> Result<(), OcrError> {
        Ok(())
    }
}

struct OcrCallHooks<'hook> {
    interceptors: &'hook dyn OcrInterceptors,
    context: OcrCallContext,
}

#[async_trait]
impl CallHooks for OcrCallHooks<'_> {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        self.interceptors
            .before_provider_request(request, &self.context)
            .await
    }

    async fn response_received(&self, body: &[u8]) -> Result<(), OcrError> {
        self.interceptors.response_received(body).await
    }
}

pub async fn perform_ocr_request(
    client: &OcrClient,
    kind: OcrConfigKind,
    mut request: PreparedOcrRequest,
    interceptors: &dyn OcrInterceptors,
) -> Result<OCRResponse, OcrError> {
    response_format(&request.optional_params)?;
    for name in kind.secret_names() {
        if let Some(secret) = client.secrets.secret(name).map_err(OcrError::Secret)? {
            let _previous = request.connection.secrets.insert(name.to_owned(), secret);
        }
    }
    let context = OcrCallContext {
        model: request.model.clone(),
        custom_llm_provider: kind.provider(),
        optional_params: request.optional_params.clone(),
        secret_fields: request
            .optional_params
            .keys()
            .filter(|name| is_secret_param(name))
            .cloned()
            .collect(),
        api_key: request.connection.api_key.clone(),
    };
    let hooks = OcrCallHooks {
        interceptors,
        context,
    };
    kind.ocr(client, &request, &hooks).await
}

pub(crate) fn is_secret_param(name: &str) -> bool {
    matches!(
        name,
        "azure_ad_token"
            | "client_secret"
            | "azure_federated_token_file"
            | "vertex_credentials"
            | "vertex_ai_credentials"
            | "aws_secret_access_key"
            | "aws_session_token"
            | "aws_web_identity_token"
    )
}
