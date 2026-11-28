use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::saml_provider::fetch::{
    FETCH_TIMEOUT_SECONDS, MAX_REDIRECTS, MetadataFetchError, MetadataFetcher,
};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse, base_builder};
use betterlite_http::tls::{TlsError, apply_policy};
use http::header::LOCATION;
use tokio::time::timeout;
use url::Url;

pub struct ProxiedMetadataFetcher {
    verified: ProxiedClient,
    unverified: ProxiedClient,
}

impl ProxiedMetadataFetcher {
    pub fn new(policy: &Arc<ProxyPolicy>) -> Result<Self, TlsError> {
        Ok(Self {
            verified: build_client(policy, Verification::Enabled)?,
            unverified: build_client(policy, Verification::Disabled)?,
        })
    }
}

fn build_client(
    policy: &Arc<ProxyPolicy>,
    verification: Verification,
) -> Result<ProxiedClient, TlsError> {
    let tls = TlsPolicy {
        verification,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-saml".to_owned(),
    };
    let builder = apply_policy(base_builder().http1_only(), &tls)?;
    ProxiedClient::build(builder, policy, TransportLimits::DEFAULT).map_err(TlsError::Http)
}

// OneLogin_Saml2_Utils.fetch_metadata: urlopen follows the redirects itself, then only a 200 is read
#[async_trait]
impl MetadataFetcher for ProxiedMetadataFetcher {
    async fn fetch(
        &self,
        url: &str,
        verify_certificate: bool,
    ) -> Result<Vec<u8>, MetadataFetchError> {
        let client = if verify_certificate {
            &self.verified
        } else {
            &self.unverified
        };
        let mut current = url.to_owned();
        for _attempt in 0..=MAX_REDIRECTS {
            let request = client
                .client
                .get(&current)
                .build()
                .map_err(HttpError::Request)
                .map_err(MetadataFetchError::Request)?;
            let answered = timeout(
                Duration::from_secs(FETCH_TIMEOUT_SECONDS),
                client.execute_buffered(request, client.limits.body_bytes),
            )
            .await
            .map_err(|_elapsed| MetadataFetchError::Request(HttpError::Timeout))?
            .map_err(MetadataFetchError::Request)?;
            match answered.status {
                200 => return Ok(answered.body),
                300..=399 => current = next_location(&current, &answered)?,
                status => return Err(MetadataFetchError::Status { status }),
            }
        }
        Err(MetadataFetchError::TooManyRedirects {
            limit: MAX_REDIRECTS,
        })
    }
}

fn next_location(current: &str, answered: &UpstreamResponse) -> Result<String, MetadataFetchError> {
    let location = answered
        .headers
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(MetadataFetchError::Status {
            status: answered.status,
        })?;
    Url::parse(current)
        .and_then(|base| base.join(location))
        .map(String::from)
        .map_err(|_error| MetadataFetchError::InvalidUrl)
}
