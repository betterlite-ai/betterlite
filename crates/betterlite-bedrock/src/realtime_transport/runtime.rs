pub mod error;
pub mod stream;

use std::error::Error as StdError;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use aws_credential_types::Credentials as SdkCredentials;
use aws_sdk_bedrockruntime::config::retry::RetryConfig;
use aws_sdk_bedrockruntime::config::{BehaviorVersion, Region};
use aws_sdk_bedrockruntime::{Client, Config};
use aws_smithy_runtime_api::client::http::{HttpClient, SharedHttpClient};
use secrecy::ExposeSecret as _;

use crate::aws_settings::sources::Credentials;
use crate::realtime_events::stream::{
    BedrockBidirectionalStream, ClosableBedrockRuntimeClient, StreamError,
};
use crate::realtime_transport::error::ProviderClass;
use error::SdkStreamError;
use stream::SdkStream;

// What the SDK config is built from once the owned preparation and credential chain have answered
pub struct RuntimeConfig {
    pub endpoint_uri: String,
    pub region: String,
    pub credentials: Credentials,
}

pub trait RealtimeRuntime: Send + Sync {
    type Error: StdError + Send + Sync + 'static + ProviderClass;

    fn connect(
        &self,
        config: RuntimeConfig,
    ) -> impl Future<
        Output = Result<Box<dyn RealtimeClient<Error = Self::Error>>, StreamError<Self::Error>>,
    > + Send;
}

#[async_trait]
pub trait RealtimeClient: Send + Sync {
    type Error: StdError + Send + Sync + 'static + ProviderClass;

    fn closable(&self) -> &dyn ClosableBedrockRuntimeClient<Error = Self::Error>;

    async fn invoke(
        &self,
        model_id: &str,
    ) -> Result<Box<dyn BedrockBidirectionalStream<Error = Self::Error>>, StreamError<Self::Error>>;
}

// The production runtime: the injected connector, explicit credentials, region and endpoint, retries off
pub struct SdkRuntime {
    http: SharedHttpClient,
}

impl SdkRuntime {
    #[must_use]
    pub fn new<H>(http: H) -> Self
    where
        H: HttpClient + 'static,
    {
        Self {
            http: SharedHttpClient::new(http),
        }
    }

    pub(crate) fn config(&self, config: &RuntimeConfig) -> Config {
        let credentials = SdkCredentials::new(
            config.credentials.access_key_id.expose_secret(),
            config.credentials.secret_access_key.expose_secret(),
            config
                .credentials
                .session_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            None,
            "betterlite-realtime",
        );
        Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .credentials_provider(credentials)
            .endpoint_url(config.endpoint_uri.clone())
            .http_client(self.http.clone())
            .retry_config(RetryConfig::disabled())
            .build()
    }
}

impl RealtimeRuntime for SdkRuntime {
    type Error = SdkStreamError;

    fn connect(
        &self,
        config: RuntimeConfig,
    ) -> impl Future<
        Output = Result<Box<dyn RealtimeClient<Error = Self::Error>>, StreamError<Self::Error>>,
    > + Send {
        let sdk = Client::from_conf(self.config(&config));
        let client: Box<dyn RealtimeClient<Error = SdkStreamError>> = Box::new(SdkRuntimeClient {
            client: Mutex::new(Some(sdk)),
        });
        ready(Ok(client))
    }
}

struct SdkRuntimeClient {
    client: Mutex<Option<Client>>,
}

#[async_trait]
impl ClosableBedrockRuntimeClient for SdkRuntimeClient {
    type Error = SdkStreamError;

    async fn close(&self) -> Result<(), StreamError<SdkStreamError>> {
        let client = self
            .client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(client);
        Ok(())
    }
}

#[async_trait]
impl RealtimeClient for SdkRuntimeClient {
    type Error = SdkStreamError;

    fn closable(&self) -> &dyn ClosableBedrockRuntimeClient<Error = SdkStreamError> {
        self
    }

    async fn invoke(
        &self,
        model_id: &str,
    ) -> Result<
        Box<dyn BedrockBidirectionalStream<Error = SdkStreamError>>,
        StreamError<SdkStreamError>,
    > {
        let client = self
            .client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(open) = client else {
            return Err(error::closed_stream());
        };
        let stream = SdkStream::open(&open, model_id).await?;
        Ok(Box::new(stream))
    }
}
