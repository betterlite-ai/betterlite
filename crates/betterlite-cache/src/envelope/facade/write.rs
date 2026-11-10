use std::collections::BTreeMap;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};

use super::{Dynamic, ResponseCache, WriteError};
use crate::backend::Ttl;
use crate::envelope::request::EnvelopeRequest;
use crate::envelope::{RESPONSE_FIELD, TIMESTAMP_FIELD};

pub(super) struct PreparedWrite {
    pub(super) key: String,
    pub(super) envelope: CacheValue,
    pub(super) ttl: Ttl,
}

pub(super) fn prepare<L, C>(
    cache: &ResponseCache<L, C>,
    request: &mut EnvelopeRequest,
    result: &CacheValue,
) -> Result<PreparedWrite, WriteError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let key = cache.resolve_key(request)?;
    // The cache control ttl wins over Cache.ttl, and the integer control value becomes the backend's f64 ttl
    let control_ttl = request
        .control
        .ttl
        .and_then(|seconds| serde_json::Number::from(seconds).as_f64());
    let ttl = control_ttl.or(cache.settings.ttl);
    let mut fields = BTreeMap::new();
    fields.insert(
        TIMESTAMP_FIELD.to_owned(),
        CacheValue::Float((cache.clock)()),
    );
    fields.insert(RESPONSE_FIELD.to_owned(), result.clone());
    Ok(PreparedWrite {
        key,
        envelope: CacheValue::Object(fields),
        ttl,
    })
}

pub(super) async fn set_through<L, C>(
    cache: &ResponseCache<L, C>,
    dynamic: Dynamic<'_>,
    request: &mut EnvelopeRequest,
    result: &CacheValue,
) -> Result<(), WriteError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let prepared = prepare(cache, request, result)?;
    dynamic
        .unwrap_or_else(|| cache.backend.as_ref())
        .async_set_cache(&prepared.key, prepared.envelope, prepared.ttl)
        .await
        .map_err(|source| WriteError::Backend { source })
}
