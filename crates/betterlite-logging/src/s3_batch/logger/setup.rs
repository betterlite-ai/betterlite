use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;

use crate::s3_batch::flush::FlushSettings;
use crate::s3_batch::logger::{InitError, S3BatchDeps};
use crate::s3_batch::upload_bounded::UploadLimiter;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::transport::{S3Transport, ThrottleSink};
use crate::s3_transport::url_construction::{EndpointInputs, S3Endpoint, resolve_endpoint};
use crate::s3_v2::{Feature, S3Parameters};

const BATCH_SIZE_VARIABLE: &str = "DEFAULT_S3_BATCH_SIZE";
const FLUSH_INTERVAL_VARIABLE: &str = "DEFAULT_S3_FLUSH_INTERVAL_SECONDS";
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
    let seconds = positive_setting(
        environment,
        FLUSH_INTERVAL_VARIABLE,
        DEFAULT_FLUSH_INTERVAL_SECONDS,
    );
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
            tracing::warn!(
                setting,
                value = %text,
                fallback,
                "s3 logging: setting is not a positive integer, using the default"
            );
            fallback
        }
    }
}

pub(super) fn transport(
    parameters: &S3Parameters<()>,
    deps: &S3BatchDeps,
) -> Result<(S3Transport, Arc<UploadLimiter>), InitError> {
    let endpoint = endpoint(parameters, deps.environment.as_ref())?;
    let adaptive = parameters.features.contains(&Feature::AdaptiveConcurrency);
    let uploads = Arc::new(UploadLimiter::new(
        parameters.max_concurrent_uploads,
        adaptive.then_some(parameters.max_adaptive_concurrency),
    )?);
    let throttle = adaptive.then(|| -> Arc<dyn ThrottleSink> {
        Arc::new(LimiterSink {
            limiter: Arc::clone(&uploads),
            observer: deps.throttle_observer.clone(),
        })
    });
    let transport = S3Transport {
        client: Arc::clone(&deps.client),
        endpoint,
        credentials: Arc::clone(&deps.credentials),
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
