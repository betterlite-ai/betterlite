use crate::s3_batch::flush::FlushSettings;
use crate::s3_batch::logger::{InitError, S3BatchDeps};
use crate::s3_batch::upload_bounded::UploadLimiter;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::{Credentials, StaticCredentials};
use crate::s3_transport::transport::{S3Transport, ThrottleSink};
use crate::s3_transport::url_construction::{EndpointInputs, S3Endpoint, resolve_endpoint};
use crate::s3_v2::connection::Credentials as ConnectionCredentials;
use crate::s3_v2::{Feature, S3Parameters};
use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use std::sync::Arc;
use std::time::Duration;
const BATCH_SIZE_VARIABLE: &str = "DEFAULT_S3_BATCH_SIZE";
const FLUSH_VARIABLE: &str = "DEFAULT_S3_FLUSH_INTERVAL_SECONDS";
const DEFAULT_BATCH_SIZE: usize = 512;
const DEFAULT_FLUSH_INTERVAL_SECONDS: usize = 10;
#[derive(Debug)]
struct LimiterSink {
    limiter: Arc<UploadLimiter>,
    observer: Option<Arc<dyn ThrottleSink>>,
}
impl ThrottleSink for LimiterSink {
    fn record(&self, throttled: bool) {
        self.limiter.record(throttled);
        if let Some(observer) = &self.observer {
            observer.record(throttled);
        }
    }
}
// the profile, AssumeRole and web-identity flows need a credential chain no logging task has
#[must_use]
pub(super) fn unsupported_flow(credentials: &ConnectionCredentials) -> Option<&'static str> {
    [
        (
            "s3_aws_profile_name",
            credentials.aws_profile_name.is_some(),
        ),
        ("s3_aws_role_name", credentials.aws_role_name.is_some()),
        (
            "s3_aws_web_identity_token",
            credentials.aws_web_identity_token.is_some(),
        ),
        (
            "s3_aws_sts_endpoint",
            credentials.aws_sts_endpoint.is_some(),
        ),
    ]
    .into_iter()
    .find_map(|(setting, configured)| configured.then_some(setting))
}
fn secret(
    configured: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
    variable: &str,
) -> Option<SecretString> {
    configured.cloned().or_else(|| {
        environment
            .lookup(variable)
            .filter(|value| !value.is_empty())
            .map(SecretString::from)
    })
}
// the boto default chain (shared credentials file, instance and container roles) is not built
fn configured_credentials(
    source: &ConnectionCredentials,
    environment: &dyn EnvironmentLookup,
) -> Result<Arc<StaticCredentials>, InitError> {
    let access_key_id = secret(
        source.aws_access_key_id.as_ref(),
        environment,
        "AWS_ACCESS_KEY_ID",
    );
    let secret_access_key = secret(
        source.aws_secret_access_key.as_ref(),
        environment,
        "AWS_SECRET_ACCESS_KEY",
    );
    let (Some(access_key_id), Some(secret_access_key)) = (access_key_id, secret_access_key) else {
        return Err(InitError::MissingCredentials);
    };
    Ok(Arc::new(StaticCredentials {
        credentials: Credentials {
            access_key_id,
            secret_access_key,
            session_token: secret(
                source.aws_session_token.as_ref(),
                environment,
                "AWS_SESSION_TOKEN",
            ),
        },
    }))
}

fn endpoint(
    parameters: &S3Parameters<()>,
    environment: &dyn EnvironmentLookup,
) -> Result<S3Endpoint, S3TransportError> {
    resolve_endpoint(
        EndpointInputs {
            bucket: parameters.connection.bucket_name.as_deref(),
            region: parameters.connection.region_name.as_deref(),
            endpoint_url: parameters.connection.endpoint_url.as_deref(),
            virtual_hosted_style: parameters.features.contains(&Feature::VirtualHostedStyle),
        },
        environment,
    )
}
pub(super) fn schedule(environment: &dyn EnvironmentLookup) -> (usize, Duration) {
    let batch_size = positive_setting(environment, BATCH_SIZE_VARIABLE, DEFAULT_BATCH_SIZE);
    let seconds = positive_setting(environment, FLUSH_VARIABLE, DEFAULT_FLUSH_INTERVAL_SECONDS);
    (
        batch_size,
        Duration::from_secs(u64::try_from(seconds).unwrap_or(10)),
    )
}
fn positive_setting(
    environment: &dyn EnvironmentLookup,
    setting: &'static str,
    fallback: usize,
) -> usize {
    let Some(text) = environment.lookup(setting) else {
        return fallback;
    };
    match text.trim().parse::<usize>() {
        Ok(value) if value > 0 => value,
        _ => {
            tracing::warn!(setting, value = %text, fallback, "s3 logging: setting is not a positive integer");
            fallback
        }
    }
}
pub(super) fn transport(
    parameters: &S3Parameters<()>,
    deps: &S3BatchDeps,
) -> Result<(S3Transport, Arc<UploadLimiter>), InitError> {
    let adaptive = parameters.features.contains(&Feature::AdaptiveConcurrency);
    let uploads = Arc::new(UploadLimiter::new(
        parameters.max_concurrent_uploads,
        adaptive.then_some(
            parameters
                .max_adaptive_concurrency
                .max(parameters.max_concurrent_uploads),
        ),
    )?);
    let throttle = adaptive.then(|| -> Arc<dyn ThrottleSink> {
        Arc::new(LimiterSink {
            limiter: Arc::clone(&uploads),
            observer: deps.throttle_observer.clone(),
        })
    });
    let transport = S3Transport {
        client: Arc::clone(&deps.client),
        endpoint: endpoint(parameters, deps.environment.as_ref())?,
        credentials: configured_credentials(
            &parameters.connection.credentials,
            deps.environment.as_ref(),
        )?,
        encryption: parameters.encryption.clone(),
        drop_on_terminal_error: parameters.features.contains(&Feature::DropOnTerminalError),
        throttle,
        clock: Arc::clone(&deps.clock),
    };
    Ok((transport, uploads))
}
pub(super) fn flush_settings(
    parameters: &S3Parameters<()>,
    cold_storage_logger: Option<String>,
) -> FlushSettings {
    FlushSettings {
        max_retry_age: parameters.max_retry_age_seconds,
        max_queue_size: parameters.max_queue_size,
        batch_file_upload: parameters.features.contains(&Feature::BatchFileUpload),
        cold_storage_logger,
        max_concurrent_uploads: parameters.max_concurrent_uploads,
    }
}
