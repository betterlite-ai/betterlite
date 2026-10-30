use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;

use crate::s3_transport::encryption::ServerSideEncryption;
use crate::s3_transport::option_value::OptionValue;
use crate::s3_transport::options::{PartitionGranularity, resolve_partition_granularity};

pub mod connection;
pub mod events;
mod features;
pub mod flag;
pub mod parameters;
mod redact;
pub mod source;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    UseSsl,
    TeamPrefix,
    KeyPrefix,
    StripBase64,
    VirtualHostedStyle,
    DropOnTerminalError,
    AdaptiveConcurrency,
    BatchFileUpload,
}

#[derive(Clone, Debug)]
pub struct S3Parameters<Config: Default> {
    pub connection: connection::Connection<Config>,
    pub features: BTreeSet<Feature>,
    pub encryption: Option<ServerSideEncryption>,
    pub max_concurrent_uploads: usize,
    pub max_queue_size: usize,
    pub max_retry_age_seconds: Option<Duration>,
    pub max_adaptive_concurrency: usize,
}

#[derive(Debug, Default)]
pub struct PartitionGranularityCache {
    raw: Option<OptionValue>,
    resolved: Option<PartitionGranularity>,
}

impl PartitionGranularityCache {
    #[must_use]
    pub fn resolve(
        &mut self,
        configured: Option<&OptionValue>,
        environment: &dyn EnvironmentLookup,
    ) -> PartitionGranularity {
        let raw = configured.cloned().or_else(|| {
            environment
                .lookup("S3_PARTITION_GRANULARITY")
                .map(OptionValue::Text)
        });
        if self.raw == raw
            && let Some(resolved) = self.resolved
        {
            return resolved;
        }
        let resolved = resolve_partition_granularity(raw.as_ref(), environment);
        self.raw = raw;
        self.resolved = Some(resolved);
        resolved
    }
}
