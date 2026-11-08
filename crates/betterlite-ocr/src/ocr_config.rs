pub mod connection;
pub mod decode;
use crate::json::{Json, Object};
use crate::ocr_auth::AwsSigningContext;
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::{
    CallHooks, OcrCallContext, OcrClient, read_response_bytes, transform_request_body,
};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use async_trait::async_trait;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use connection::{OcrConnection, OcrCredentialInputs, OcrInputSource, ResolvedOcrCredentials};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
pub const OCR_RESPONSE_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const OCR_INLINE_MAX_BYTES: usize = 50 * 1024 * 1024;
pub const OCR_MAX_FETCH_REDIRECTS: usize = 10;
pub const OCR_POLL_RETRY_SECS: u64 = 2;
const HEALTH_CHECK_PDF_DATA_URI: &str = "data:application/pdf;base64,JVBERi0xLjQKJeLjz9MKMyAwIG9iago8PC9UeXBlIC9QYWdlCi9QYXJlbnQgMSAwIFIKL01lZGlhQm94IFswIDAgNjEyIDc5Ml0KL0NvbnRlbnRzIDQgMCBSCi9SZXNvdXJjZXMgPDwvRm9udCA8PC9GMSAyIDAgUj4+Pj4+PgplbmRvYmoKNCAwIG9iago8PC9MZW5ndGggNDQ+PgpzdHJlYW0KQlQKL0YxIDI0IFRmCjEwMCA3MDAgVGQKKHRlc3QpIFRqCkVUCmVuZHN0cmVhbQplbmRvYmoKMiAwIG9iago8PC9UeXBlIC9Gb250Ci9TdWJ0eXBlIC9UeXBlMQovQmFzZUZvbnQgL0hlbHZldGljYT4+CmVuZG9iagoxIDAgb2JqCjw8L1R5cGUgL1BhZ2VzCi9LaWRzIFszIDAgUl0KL0NvdW50IDE+PgplbmRvYmoKNSAwIG9iago8PC9UeXBlIC9DYXRhbG9nCi9QYWdlcyAxIDAgUj4+CmVuZG9iagp0cmFpbGVyCjw8L1NpemUgNgovUm9vdCA1IDAgUj4+CnN0YXJ0eHJlZgozMjQKJSVFT0Y=";
pub type Secrets = BTreeMap<String, SecretString>;
#[async_trait]
pub trait AzureAdTokenProvider: Send + Sync {
    async fn token(&self) -> Result<SecretString, OcrError>;
}
pub struct PreparedOcrRequest {
    pub model: String,
    pub document: OcrDocument,
    pub connection: OcrConnection,
    pub caller_document: bool,
    pub optional_params: Object,
    pub input_sources: BTreeMap<String, OcrInputSource>,
    pub azure_ad_token_provider: Option<Arc<dyn AzureAdTokenProvider>>,
}
pub trait OcrEnvironment: Send + Sync {
    fn headers(&self) -> &[(String, String)];
    fn signer(&self) -> Option<AwsSigningContext<'_>> {
        None
    }
}
impl OcrEnvironment for Vec<(String, String)> {
    fn headers(&self) -> &[(String, String)] {
        self
    }
}
#[derive(Clone, Copy)]
pub struct OcrRequestContext<'call> {
    pub client: &'call OcrClient,
    pub connection: &'call OcrConnection,
}
#[derive(Clone, Copy)]
pub struct OcrTransform<'transform, P> {
    pub model: &'transform str,
    pub optional_params: &'transform P,
    pub headers: &'transform [(String, String)],
}
#[derive(Clone, Copy)]
pub struct OcrResponseContext<'call> {
    pub client: &'call OcrClient,
    pub connection: &'call OcrConnection,
    pub hooks: &'call dyn CallHooks,
    pub request_format: RequestFormat,
    pub url: &'call str,
    pub headers: &'call [(String, String)],
}
#[derive(Debug)]
pub struct DecodedOcrResponse<T> {
    pub data: T,
    pub native: Option<Json>,
    pub text: String,
}
pub trait OcrProviderConfig: Send + Sync + Sized + 'static {
    type OcrParams: Send + Sync;
    type ProviderRequest: Serialize + Send;
    type Environment: OcrEnvironment;
    fn get_supported_ocr_params(&self, _model: &str) -> &'static [&'static str] {
        &[]
    }
    fn get_api_key_env_var(&self) -> Option<&'static str> {
        None
    }
    fn secret_names(&self) -> Vec<&'static str>;
    fn resolve_connection_params(&self, inputs: OcrCredentialInputs) -> ResolvedOcrCredentials {
        ResolvedOcrCredentials {
            api_key: inputs
                .dynamic_api_key
                .filter(|value| !value.value.expose_secret().is_empty())
                .or(inputs.api_key),
            api_base: inputs
                .dynamic_api_base
                .filter(|value| !value.value.is_empty())
                .or(inputs.api_base),
        }
    }
    fn get_health_check_document(&self) -> OcrDocument {
        OcrDocument::DocumentUrl {
            document_url: HEALTH_CHECK_PDF_DATA_URI.to_owned(),
            extra: BTreeMap::new(),
        }
    }
    fn map_ocr_params(
        &self,
        non_default_params: &Object,
        model: &str,
    ) -> Result<Self::OcrParams, OcrError>;
    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
    ) -> impl Future<Output = Result<Self::Environment, OcrError>> + Send;
    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        optional_params: &Self::OcrParams,
        environment: &Self::Environment,
    ) -> Result<String, OcrError>;
    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError>;
    fn async_transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
        _context: OcrRequestContext<'_>,
    ) -> impl Future<Output = Result<Self::ProviderRequest, OcrError>> + Send {
        async move { self.transform_ocr_request(document, transform) }
    }
    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError>;
    fn async_transform_ocr_response(
        &self,
        model: &str,
        raw_response: StreamResponse,
        context: OcrResponseContext<'_>,
    ) -> impl Future<Output = Result<OCRResponse, OcrError>> + Send {
        async move {
            let bytes =
                read_response_bytes(raw_response, context.connection.max_response_bytes).await?;
            context.hooks.response_received(&bytes).await?;
            self.transform_ocr_response(model, &bytes, context.request_format)
        }
    }
    fn get_error_class(
        &self,
        error_message: String,
        status_code: u16,
        headers: Vec<(String, String)>,
    ) -> OcrError {
        OcrError::Provider {
            status: status_code,
            body: error_message,
            headers,
        }
    }
    fn validate_request_body(&self, _body: &Json) -> Result<(), OcrError> {
        Ok(())
    }
    fn prepare_request(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
        hooks: &dyn CallHooks,
    ) -> impl Future<Output = Result<OutboundRequest, OcrError>> + Send {
        async move {
            let params = self.map_ocr_params(&request.optional_params, &request.model)?;
            let environment = self.validate_environment(request, client).await?;
            let url = self.get_complete_url(request, &params, &environment)?;
            let transform = OcrTransform {
                model: &request.model,
                optional_params: &params,
                headers: environment.headers(),
            };
            let request_context = OcrRequestContext {
                client,
                connection: &request.connection,
            };
            let body = self
                .async_transform_ocr_request(request.document.clone(), transform, request_context)
                .await?;
            let call_context = OcrCallContext {
                environment: &environment,
                hooks,
            };
            transform_request_body(self, request, &url, call_context, body).await
        }
    }
}
mod tests;
