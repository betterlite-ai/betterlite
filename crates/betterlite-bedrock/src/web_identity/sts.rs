use super::error::WebIdentityError;
use crate::assume_role::expiry::{Expiration, StsCredentials, StsResponse};
use crate::aws_endpoints::sts::StsClientKwargs;
use crate::aws_settings::sources::NativeSources;
use aws_config::{BehaviorVersion, Region, retry::RetryConfig, timeout::TimeoutConfig};
use aws_credential_types::Credentials as SdkCredentials;
use aws_sdk_sts::Client;
use aws_sdk_sts::config::Builder;
use aws_sdk_sts::types::Credentials as ResponseCredentials;
use aws_types::os_shim_internal::Env;
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use std::collections::HashMap;
pub(super) async fn sts_client(
    kwargs: &StsClientKwargs,
    native: &NativeSources,
    credentials: Option<SdkCredentials>,
) -> Client {
    let mut loader = aws_config::defaults(BehaviorVersion::latest())
        .env(Env::from(HashMap::from_iter(native.environment.clone())))
        .fs(native.filesystem.clone())
        .http_client(native.client.clone())
        .time_source(native.clock.clone())
        .sleep_impl(native.sleep.clone())
        .retry_config(RetryConfig::disabled())
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(native.timeout)
                .build(),
        );
    if let Some(provider) = credentials {
        loader = loader.credentials_provider(provider);
    }
    let shared = match kwargs.region_name.as_deref() {
        Some(region) => loader.region(Region::new(region.to_owned())).load().await,
        None => loader.load().await,
    };
    let mut builder = Builder::from(&shared);
    if let Some(endpoint) = &kwargs.endpoint_url {
        builder = builder.endpoint_url(endpoint.clone());
    }
    // boto3.client("sts") without any region talks to the aws-global endpoint
    if shared.region().is_none() {
        builder = builder.region(Region::from_static("aws-global"));
    }
    Client::from_conf(builder.build())
}
pub(super) fn required_credentials<'creds>(
    required: Option<&'creds ResponseCredentials>,
    call: &'static str,
) -> Result<&'creds ResponseCredentials, WebIdentityError> {
    let credentials = required.ok_or_else(|| {
        WebIdentityError::MissingField(format!("the STS {call} response has no Credentials"))
    })?;
    for (member, value) in [
        ("AccessKeyId", credentials.access_key_id()),
        ("SecretAccessKey", credentials.secret_access_key()),
        ("SessionToken", credentials.session_token()),
    ] {
        if value.is_empty() {
            return Err(WebIdentityError::MissingField(format!(
                "the STS {call} response has no {member}"
            )));
        }
    }
    Ok(credentials)
}
pub(super) fn identity(credentials: &ResponseCredentials) -> SdkCredentials {
    SdkCredentials::new(
        credentials.access_key_id(),
        credentials.secret_access_key(),
        Some(credentials.session_token().to_owned()),
        None,
        "manual-irsa",
    )
}
pub(super) fn sts_response(
    credentials: &ResponseCredentials,
) -> Result<StsResponse, WebIdentityError> {
    required_credentials(Some(credentials), "assume role")?;
    let expiration = credentials.expiration();
    if expiration.secs() == 0 && expiration.subsec_nanos() == 0 {
        return Err(WebIdentityError::MissingField(
            "the STS assume role response has no Expiration".to_owned(),
        ));
    }
    let expiry = DateTime::<Utc>::from_timestamp(expiration.secs(), expiration.subsec_nanos())
        .ok_or(WebIdentityError::Expiry)?
        .fixed_offset();
    Ok(StsResponse {
        credentials: StsCredentials {
            access_key_id: SecretString::from(credentials.access_key_id().to_owned()),
            secret_access_key: SecretString::from(credentials.secret_access_key().to_owned()),
            session_token: SecretString::from(credentials.session_token().to_owned()),
            expiration: Expiration::Aware(expiry),
        },
    })
}
