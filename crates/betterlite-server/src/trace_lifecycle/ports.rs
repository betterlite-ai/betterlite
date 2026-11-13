use std::future::Future;
use std::sync::Arc;

use betterlite_logging::clickhouse::clickhouse_logging::storage::ClickHouseStorage;

use crate::trace_lifecycle::error::ReceiverFailure;

pub trait ReceiverFactory: Send + Sync {
    type Receiver: ReceiverHandle;
    type Error: ReceiverFailure;

    fn build(&self) -> Result<Self::Receiver, Self::Error>;
}

pub trait ReceiverHandle: Send + Sync {
    type Storage: ClickHouseStorage;
    type Error: ReceiverFailure;

    fn storage(&self) -> Arc<Self::Storage>;

    fn start(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
