use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug)]
pub struct VerifyRequest<'request> {
    pub response_xml: &'request [u8],
    pub expected_request_id: Option<&'request str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAttribute {
    pub name: String,
    pub values: Vec<String>,
}

// only values the signature or the response's own signed assertion covers reach this type
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAssertion {
    pub assertion_id: Option<String>,
    pub subject: Option<String>,
    pub name_id_format: Option<String>,
    pub session_index: Option<String>,
    pub issuer: Option<String>,
    pub audience: Vec<String>,
    pub in_response_to: Option<String>,
    pub not_on_or_after: Option<i64>,
    pub attributes: Vec<VerifiedAttribute>,
    pub signed_assertion: bool,
    pub signed_response: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedResponse {
    pub in_response_to: Option<String>,
    pub issuer: Option<String>,
    pub destination: Option<String>,
    pub assertions: Vec<VerifiedAssertion>,
    pub signed_response: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SamlVerificationError {
    #[error("the SAML signature backend is unavailable")]
    Unavailable,
    #[error("the SAML response was rejected")]
    Rejected { reason: String },
}

#[async_trait]
pub trait SamlResponseVerifier: Send + Sync {
    async fn verify(
        &self,
        request: &VerifyRequest<'_>,
    ) -> Result<VerifiedResponse, SamlVerificationError>;
}

// the toolkit's xmlsec is not part of this build, and no fallback path may accept an unsigned response
pub struct UnavailableSamlVerifier;

#[async_trait]
impl SamlResponseVerifier for UnavailableSamlVerifier {
    async fn verify(
        &self,
        _request: &VerifyRequest<'_>,
    ) -> Result<VerifiedResponse, SamlVerificationError> {
        Err(SamlVerificationError::Unavailable)
    }
}
