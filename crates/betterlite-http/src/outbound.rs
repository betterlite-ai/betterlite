use std::time::Duration;

use reqwest::{Client, RequestBuilder, Response, Url};
use secrecy::{ExposeSecret as _, SecretString};

use crate::error::HttpError;

#[derive(Clone, Debug)]
pub struct OutboundClient {
    client: Client,
}

impl OutboundClient {
    pub fn new() -> Result<Self, HttpError> {
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
        request.send().await.map_err(HttpError::Request)
    }
}
