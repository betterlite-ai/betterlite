use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Builder, Credentials, Region, SharedHttpClient};
use aws_smithy_types::retry::RetryConfig;
use aws_smithy_types::timeout::TimeoutConfig;
use betterlite_config::sections::s3_response_cache::S3Settings;
use betterlite_config::sections::s3_response_cache::options::AddressingStyle;
use secrecy::ExposeSecret as _;

pub(super) fn build(settings: &S3Settings, transport: SharedHttpClient) -> Client {
    let timeouts = TimeoutConfig::builder()
        .connect_timeout(Duration::from_secs(settings.config.connect_timeout))
        .read_timeout(Duration::from_secs(settings.config.read_timeout))
        .build();
    let mut builder = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .http_client(transport)
        .region(Region::new(settings.region_name.clone()))
        .timeout_config(timeouts)
        .retry_config(
            RetryConfig::standard()
                .with_max_attempts(settings.config.retries.max_attempts.saturating_add(1)),
        )
        .force_path_style(matches!(
            settings.config.s3.addressing_style,
            AddressingStyle::Path
        ));
    if let Some(endpoint) = &settings.endpoint_url {
        builder = builder.endpoint_url(endpoint);
    }
    if let Some(credentials) = &settings.credentials {
        builder = builder.credentials_provider(Credentials::new(
            credentials.access_key.expose_secret(),
            credentials.secret_key.expose_secret(),
            credentials
                .session_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            None,
            "betterlite-config",
        ));
    }
    Client::from_conf(builder.build())
}
