use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_ocr::ocr_auth::{
    AwsRequestSigner, AwsSigningRequest, AzureTokenSource, VertexAccessToken, VertexTokenSource,
};
use betterlite_ocr::ocr_config::PreparedOcrRequest;
use betterlite_ocr::ocr_config::connection::OcrSourced;
use betterlite_ocr::ocr_error::{OcrError, OcrSecretError};
use betterlite_ocr::ocr_http::{OcrClient, SecretSource};
use betterlite_ocr::ocr_settings::OcrSettings;
use secrecy::SecretString;

use crate::helpers::HelperError;

pub(super) fn fail(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn client() -> Result<OcrClient, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| fail("ProxyPolicyError", error.to_string()))?,
    );
    let limits = TransportLimits {
        read_timeout: Duration::from_secs(5),
        ..TransportLimits::DEFAULT
    };
    let http = ProxiedClient::build(base_builder(), &policy, limits)
        .map_err(|error| fail("HttpError", error.to_string()))?;
    let allowed = vec!["127.0.0.1".to_owned()];
    let validator = UrlValidator::new(SystemResolver, &allowed, &Verification::Enabled);
    Ok(OcrClient {
        provider_http: http.clone(),
        polling_http: http.clone(),
        document_http: http,
        document_fetcher: UserUrlFetcher {
            validator,
            validation: true,
        },
        settings: OcrSettings::default(),
        secrets: Arc::new(NoSecrets),
        azure: Arc::new(NoAzure),
        aws: Arc::new(NoAws),
        vertex: Arc::new(NoVertex),
    })
}

struct NoSecrets;

impl SecretSource for NoSecrets {
    fn secret(&self, _name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Ok(None)
    }
}

struct NoAzure;

#[async_trait]
impl AzureTokenSource for NoAzure {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        Ok(None)
    }
}

struct NoAws;

#[async_trait]
impl AwsRequestSigner for NoAws {
    async fn sign(
        &self,
        _request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        Ok(Vec::new())
    }
}

struct NoVertex;

#[async_trait]
impl VertexTokenSource for NoVertex {
    async fn token(&self, _request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError> {
        Err(OcrError::Unsupported("vertex tokens in the b6801 helper"))
    }
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| fail("RuntimeError", error.to_string()))
}
