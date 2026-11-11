use std::str::FromStr;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;

use crate::batch::logger::{
    BATCH_SIZE_VARIABLE, BatchSettings, DEFAULT_BATCH_SIZE, DEFAULT_FLUSH_INTERVAL_SECONDS,
    DEFAULT_MAX_QUEUE_SIZE, FLUSH_INTERVAL_VARIABLE,
};

use crate::datadog_llm_observability::error::DatadogLlmObsError;

use super::DatadogLlmObsInit;

// CustomBatchLogger takes a batch_size and a flush_interval and defaults both from the environment
pub(super) fn batch(
    init: &DatadogLlmObsInit,
    environment: &dyn EnvironmentLookup,
) -> Result<BatchSettings, DatadogLlmObsError> {
    let batch_size = if let Some(size) = init.batch_size {
        size
    } else {
        number::<usize>(environment, BATCH_SIZE_VARIABLE)?.unwrap_or(DEFAULT_BATCH_SIZE)
    };
    let flush_interval = match init.flush_interval_seconds {
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|_source| DatadogLlmObsError::FlushInterval { seconds })?,
        None => Duration::from_secs(
            number::<u64>(environment, FLUSH_INTERVAL_VARIABLE)?
                .unwrap_or(DEFAULT_FLUSH_INTERVAL_SECONDS),
        ),
    };
    Ok(BatchSettings {
        batch_size,
        flush_interval,
        max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
    })
}

fn number<T>(
    environment: &dyn EnvironmentLookup,
    name: &str,
) -> Result<Option<T>, DatadogLlmObsError>
where
    T: FromStr,
{
    let Some(text) = environment.lookup(name) else {
        return Ok(None);
    };
    let parsed = betterlite_env::number::<T>(name, &text)?;
    Ok(parsed)
}
