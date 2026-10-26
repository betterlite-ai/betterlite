use std::time::Duration;

use reqwest::{Client, RequestBuilder, Response, Url};
use rustls::crypto::aws_lc_rs::default_provider;
use secrecy::{ExposeSecret as _, SecretString};

use crate::error::HttpError;

pub fn install_crypto_provider() {
    let provider = default_provider();
    let _result = provider.install_default();
}

#[derive(Clone, Debug)]
pub struct OutboundClient {
    client: Client,
}

impl OutboundClient {
    pub fn new() -> Result<Self, HttpError> {
        install_crypto_provider();
        #[expect(
            clippy::disallowed_methods,
            reason = "outbound clients are centralized here so TLS and pooling are configured once"
        )]
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .map_err(HttpError::Client)?;
        Ok(Self { client })
    }

    pub fn post_json<Body>(
        &self,
        url: Url,
        api_key: Option<&SecretString>,
        timeout: Duration,
        body: &Body,
    ) -> RequestBuilder
    where
        Body: serde::Serialize,
    {
        let request = self.client.post(url).timeout(timeout).json(body);
        match api_key {
            Some(key) => request.bearer_auth(key.expose_secret()),
            None => request,
        }
    }

    pub async fn send(&self, request: RequestBuilder) -> Result<Response, HttpError> {
        let response = request.send().await;
        response.map_err(HttpError::Request)
    }
}
