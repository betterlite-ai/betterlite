use std::collections::BTreeMap;
use std::sync::Arc;

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

use super::failure;

struct Unused;

impl SecretSource for Unused {
    fn secret(&self, _name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Ok(None)
    }
}

#[async_trait]
impl AzureTokenSource for Unused {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        Ok(None)
    }
}

#[async_trait]
impl AwsRequestSigner for Unused {
    async fn sign(
        &self,
        _request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl VertexTokenSource for Unused {
    async fn token(&self, _request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError> {
        Err(OcrError::Unsupported("vertex tokens in the b6805 helper"))
    }
}

pub(super) fn ocr_client() -> Result<OcrClient, Box<crate::helpers::HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| failure(&error))?,
    );
    let http = ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT)
        .map_err(|error| failure(&error))?;
    let validator = UrlValidator::new(
        SystemResolver,
        &["127.0.0.1".to_owned()],
        &Verification::Enabled,
    );
    Ok(OcrClient {
        provider_http: http.clone(),
        polling_http: http.clone(),
        document_http: http,
        document_fetcher: UserUrlFetcher {
            validator,
            validation: true,
        },
        settings: OcrSettings::default(),
        secrets: Arc::new(Unused),
        azure: Arc::new(Unused),
        aws: Arc::new(Unused),
        vertex: Arc::new(Unused),
    })
}
