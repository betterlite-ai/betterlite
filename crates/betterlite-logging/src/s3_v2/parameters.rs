use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;

use crate::s3_transport::encryption::{ServerSideEncryption, resolve_server_side_encryption};
use crate::s3_transport::option_value::{OptionValue, integer_from_text};
use crate::s3_transport::options::{
    Bound, default_adaptive_limit, resolve_bound, resolve_max_retry_age,
};
use crate::s3_v2::source::{Provided, Source};
use crate::s3_v2::{S3Parameters, connection, features};

#[must_use]
pub fn resolve_s3_parameters<Config>(
    defaults: &Source<Config>,
    global_source: &Source<Config>,
    audit_source: Option<&Source<Config>>,
    environment: &dyn EnvironmentLookup,
) -> S3Parameters<Config>
where
    Config: Clone + Default,
{
    let source = audit_source.unwrap_or(global_source);
    let connection = connection::resolve(source, defaults);
    let retry = retry_age(
        &source.max_retry_age_seconds,
        &defaults.max_retry_age_seconds,
    );
    let encryption = encryption(source, defaults);
    let queue = resolve_bound(
        Bound::MaxQueueSize,
        defaults.max_queue_size.as_ref(),
        50_000,
    );
    S3Parameters {
        features: features::resolve(source, defaults),
        encryption,
        max_concurrent_uploads: configured_bound(
            Bound::MaxConcurrentUploads,
            source.max_concurrent_uploads.as_ref(),
            defaults.max_concurrent_uploads.as_ref(),
            default_uploads(environment),
        ),
        max_queue_size: resolve_bound(Bound::MaxQueueSize, source.max_queue_size.as_ref(), queue),
        max_retry_age_seconds: retry,
        max_adaptive_concurrency: configured_bound(
            Bound::MaxAdaptiveConcurrency,
            source.max_adaptive_concurrency.as_ref(),
            defaults.max_adaptive_concurrency.as_ref(),
            default_adaptive_limit(environment),
        ),
        connection,
    }
}

const fn prefer<'value>(
    configured: Option<&'value OptionValue>,
    fallback: Option<&'value OptionValue>,
) -> Option<&'value OptionValue> {
    match configured {
        None => fallback,
        Some(OptionValue::Text(text)) if text.is_empty() => fallback,
        Some(value) => Some(value),
    }
}

fn retry_age(source: &Provided<OptionValue>, defaults: &Provided<OptionValue>) -> Option<Duration> {
    let fallback = match defaults {
        Provided::Missing => Some(Duration::from_secs(3_600)),
        Provided::Present(value) => {
            resolve_max_retry_age(value.as_ref(), Some(Duration::from_secs(3_600)))
        }
    };
    match source {
        Provided::Missing | Provided::Present(None) => fallback,
        Provided::Present(Some(OptionValue::Text(text))) if text.is_empty() => fallback,
        Provided::Present(Some(value)) => resolve_max_retry_age(Some(value), fallback),
    }
}

fn configured_bound(
    bound: Bound,
    source: Option<&OptionValue>,
    defaults: Option<&OptionValue>,
    fallback: usize,
) -> usize {
    let chosen = prefer(source, defaults);
    resolve_bound(bound, chosen, fallback)
}

fn encryption<Config>(
    source: &Source<Config>,
    defaults: &Source<Config>,
) -> Option<ServerSideEncryption>
where
    Config: Default,
{
    let algorithm = source
        .server_side_encryption
        .as_ref()
        .filter(|value| features::truthy(Some(value)))
        .or(defaults.server_side_encryption.as_ref());
    let kms_key = source
        .sse_kms_key_id
        .as_ref()
        .filter(|value| features::truthy(Some(value)))
        .or(defaults.sse_kms_key_id.as_ref());
    resolve_server_side_encryption(algorithm, kms_key)
}

fn default_uploads(environment: &dyn EnvironmentLookup) -> usize {
    const DEFAULT_UPLOADS: usize = 16;
    let configured = environment.lookup("DEFAULT_S3_MAX_CONCURRENT_UPLOADS");
    configured
        .as_deref()
        .filter(|text| !text.contains('.'))
        .and_then(integer_from_text)
        .and_then(|number| usize::try_from(number).ok())
        .unwrap_or(DEFAULT_UPLOADS)
}
