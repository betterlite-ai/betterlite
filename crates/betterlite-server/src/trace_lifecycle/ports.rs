use std::future::Future;
use std::sync::Arc;

use betterlite_logging::clickhouse::clickhouse_logging::storage::ClickHouseStorage;

use crate::trace_lifecycle::error::TraceLifecycleError;

pub trait ReceiverFactory: Send + Sync {
    type Receiver: ReceiverHandle;

    fn build(&self) -> Result<Self::Receiver, TraceLifecycleError>;
}

pub trait ReceiverHandle: Send + Sync {
    type Storage: ClickHouseStorage;

    fn storage(&self) -> Arc<Self::Storage>;

    fn start(&self) -> impl Future<Output = Result<(), TraceLifecycleError>> + Send;
}
