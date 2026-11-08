use std::sync::{Arc, Mutex};

use betterlite_context::local_ttl_cache::LocalTtlCache;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use self::environment::WatsonxEnvironment;
use self::error::WatsonxError;
use self::headers::{token_from_params, zen_api_key_from_params};
use self::iam::{
    EXPIRY_BUFFER_OFFSET, cached_token, decode_iam_token_response, get_watsonx_iam_url,
    iam_request, resolve_api_key, store_token,
};
use self::transport::WatsonxTransport;
use self::types::WatsonxParams;

pub mod credentials;
pub mod environment;
pub mod error;
pub mod headers;
pub mod iam;
pub mod payload;
pub mod transport;
pub mod types;
pub mod urls;

mod tests;

// InMemoryCache's own defaults: 200 entries, 600 second default TTL, 1024 KiB per item
const IAM_CACHE_ITEM_KIB: i64 = 1024;

pub struct WatsonxAuth<T> {
    pub environment: WatsonxEnvironment,
    pub transport: T,
    pub cache: Mutex<LocalTtlCache>,
}

impl<T: WatsonxTransport> WatsonxAuth<T> {
    pub fn new(
        environment: WatsonxEnvironment,
        transport: T,
        clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    ) -> Result<Self, WatsonxError> {
        let cache = LocalTtlCache::with_clock(None, None, None, IAM_CACHE_ITEM_KIB, clock)
            .map_err(WatsonxError::Cache)?;
        Ok(Self {
            environment,
            transport,
            cache: Mutex::new(cache),
        })
    }

    pub async fn generate_iam_token(
        &self,
        api_key: Option<&SecretString>,
    ) -> Result<SecretString, WatsonxError> {
        if let Some(key) = api_key
            && let Some(cached) = cached_token(self, key.expose_secret())?
        {
            return Ok(cached);
        }
        let resolved =
            resolve_api_key(api_key, &self.environment).ok_or(WatsonxError::IamApiKeyRequired)?;
        let url = get_watsonx_iam_url(&self.environment);
        let request = iam_request(&url, &resolved)?;
        tracing::debug!(url = %url, headers = ?request.headers, "calling IBM /identity/token to retrieve IAM token");
        let response = self
            .transport
            .send_buffered(request)
            .await
            .map_err(WatsonxError::IamTransport)?;
        if !(200..300).contains(&response.status) {
            return Err(WatsonxError::IamStatus {
                status: response.status,
                headers: response.headers,
            });
        }
        let reply = decode_iam_token_response(&response.body)?;
        let ttl = reply.expires_in.mul_add(1.0, EXPIRY_BUFFER_OFFSET);
        store_token(self, resolved.expose_secret(), &reply.access_token, ttl)?;
        Ok(reply.access_token)
    }

    pub async fn generate_watsonx_token(
        &self,
        api_key: Option<&SecretString>,
        token: Option<&SecretString>,
    ) -> Result<SecretString, WatsonxError> {
        match token {
            Some(given) => Ok(given.clone()),
            None => self.generate_iam_token(api_key).await,
        }
    }

    pub async fn validate_environment(
        &self,
        headers: HeaderMap,
        optional_params: &mut WatsonxParams,
        api_key: Option<&SecretString>,
    ) -> Result<HeaderMap, WatsonxError> {
        let mut resolved = HeaderMap::new();
        resolved.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        resolved.insert(ACCEPT, HeaderValue::from_static("application/json"));
        if headers.contains_key(AUTHORIZATION) {
            for (name, value) in &headers {
                resolved.insert(name, value.clone());
            }
            return Ok(resolved);
        }
        let requested_token = token_from_params(optional_params, &self.environment)?;
        let requested_zen = zen_api_key_from_params(optional_params, &self.environment)?;
        let authorization = if let Some(token_value) = requested_token {
            format!("Bearer {}", token_value.expose_secret())
        } else if let Some(zen_value) = requested_zen {
            format!("ZenApiKey {}", zen_value.expose_secret())
        } else {
            let iam_token = self.generate_iam_token(api_key).await?;
            format!("Bearer {}", iam_token.expose_secret())
        };
        let mut value =
            HeaderValue::try_from(authorization).map_err(|source| WatsonxError::InvalidHeader {
                header: "Authorization",
                source,
            })?;
        value.set_sensitive(true);
        resolved.insert(AUTHORIZATION, value);
        Ok(resolved)
    }
}
