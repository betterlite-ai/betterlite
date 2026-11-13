use crate::json::Object;
use crate::ocr_config::connection::OcrSourced;
use crate::ocr_config::{PreparedOcrRequest, Secrets};
use crate::ocr_error::OcrError;
use async_trait::async_trait;
use secrecy::SecretString;
#[async_trait]
pub trait AzureTokenSource: Send + Sync {
    async fn token(
        &self,
        request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError>;
}
#[derive(Clone)]
pub struct VertexAccessToken {
    pub token: SecretString,
    pub project: Option<String>,
}
#[async_trait]
pub trait VertexTokenSource: Send + Sync {
    async fn token(&self, request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError>;
}
#[derive(Clone, Copy)]
pub struct AwsSigningContext<'signing> {
    pub signer: &'signing dyn AwsRequestSigner,
    pub region: &'signing str,
    pub service: &'signing str,
}
pub struct AwsSigningRequest<'signing> {
    pub method: &'signing str,
    pub url: &'signing str,
    pub headers: &'signing [(String, String)],
    pub body: &'signing [u8],
    pub service: &'signing str,
    pub region: &'signing str,
    pub optional_params: &'signing Object,
    pub secrets: &'signing Secrets,
}
#[async_trait]
pub trait AwsRequestSigner: Send + Sync {
    async fn sign(
        &self,
        request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError>;
}
