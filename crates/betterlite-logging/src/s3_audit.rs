use std::sync::PoisonError;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::s3_batch::Element;
use crate::s3_batch::flush::FlushError;
use crate::s3_batch::logger::S3BatchLogger;
use crate::s3_transport::options::PartitionGranularity;

pub mod dedicated;
pub mod element;
mod tests;

use element::{AuditElementError, audit_element};

#[derive(Debug, thiserror::Error)]
pub enum AuditLogError {
    #[error("the S3 audit record could not be serialized")]
    Serialize(#[source] serde_json::Error),
    #[error(transparent)]
    Element(#[from] AuditElementError),
    #[error(transparent)]
    Flush(#[from] FlushError),
}

// audit records share the model-event queue, and a record that fills a batch flushes it at once
pub(crate) async fn append_audit_element(
    logger: &S3BatchLogger,
    element: Element,
) -> Result<(), AuditLogError> {
    logger.queue.enqueue(element);
    let queued = logger
        .queue
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entries
        .len();
    if queued >= logger.batch_size {
        logger.flush_queue().await?;
    }
    Ok(())
}

// the granularity the logger's partition cache resolves now, from its configured value and the environment
pub(crate) fn current_granularity(logger: &S3BatchLogger) -> PartitionGranularity {
    let mut cache = logger
        .partition_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    cache.resolve(
        logger.parameters.connection.partition_granularity.as_ref(),
        logger.environment.as_ref(),
    )
}

pub(crate) fn audit_record<Audit: Serialize>(
    logger: &S3BatchLogger,
    audit_log: &Audit,
    now: DateTime<Utc>,
) -> Result<Element, AuditLogError> {
    let encoded = serde_json::to_value(audit_log).map_err(AuditLogError::Serialize)?;
    let record: Json = serde_json::from_value(encoded).map_err(AuditLogError::Serialize)?;
    let element = audit_element(
        logger
            .parameters
            .connection
            .path
            .as_deref()
            .unwrap_or_default(),
        current_granularity(logger),
        &record,
        now,
    )?;
    Ok(element)
}
