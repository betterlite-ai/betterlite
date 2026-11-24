use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::pool::{ClientPool, PoolError};
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

mod tests;

// One client per call, built from the session's environment: the secrets LiteLLM would read from os.environ.
pub(crate) fn build(
    pool: &ClientPool,
    tls: &TlsPolicy,
    proxy: &Arc<ProxyPolicy>,
    settings: OcrSettings,
    environment: &BTreeMap<String, String>,
) -> Result<OcrClient, PoolError> {
    let client = pool.client(tls, proxy)?;
    Ok(OcrClient {
        provider_http: client.clone(),
        polling_http: client.clone(),
        document_http: client,
        document_fetcher: UserUrlFetcher {
            validator: UrlValidator::new(SystemResolver, &[], &tls.verification),
            validation: true,
        },
        settings,
        secrets: Arc::new(EnvironmentSecrets {
            environment: environment.clone(),
        }),
        azure: Arc::new(NoAzureToken),
        aws: Arc::new(UnsignedAws),
        vertex: Arc::new(NoVertexToken),
    })
}

// get_secret_str: the environment value, with an empty one counting as unset
struct EnvironmentSecrets {
    environment: BTreeMap<String, String>,
}

impl SecretSource for EnvironmentSecrets {
    fn secret(&self, name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Ok(self
            .environment
            .get(name)
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.clone())))
    }
}

// Entra ID needs azure-identity, which betterlite does not port, so a caller token is the only Azure credential.
struct NoAzureToken;

#[async_trait]
impl AzureTokenSource for NoAzureToken {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        Ok(None)
    }
}

// SigV4 signing is not ported; the call reaches the provider unsigned and its 403 is the answer.
struct UnsignedAws;

#[async_trait]
impl AwsRequestSigner for UnsignedAws {
    // the empty header list is the whole behavior, so every mutant of this body either equals it or invents a header
    #[cfg_attr(test, mutants::skip)]
    async fn sign(
        &self,
        _request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        Ok(Vec::new())
    }
}

// A Vertex access token needs google-auth; without it the provider answers its own auth failure.
struct NoVertexToken;

#[async_trait]
impl VertexTokenSource for NoVertexToken {
    async fn token(&self, _request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError> {
        Err(OcrError::Unsupported("vertex access tokens"))
    }
}
