use std::sync::Arc;

use betterlite_logging::clickhouse::clickhouse_logging::storage::ClickHouseStorage;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::identity::CallbackIdentity;

use crate::trace_lifecycle::REGISTRATION_LISTS;
use crate::trace_lifecycle::ports::ReceiverHandle;
use crate::trace_lifecycle::spend_logger::TraceSpendLogger;

pub struct Started<Receiver: ReceiverHandle> {
    pub receiver: Arc<Receiver>,
    pub session: TraceSession<Receiver::Storage>,
}

pub struct TraceSession<Storage: ClickHouseStorage> {
    pub(crate) logger: Arc<TraceSpendLogger<Storage>>,
    pub(crate) identity: CallbackIdentity,
}

impl<Storage: ClickHouseStorage> TraceSession<Storage> {
    pub async fn close(self, registry: &mut CallbackRegistry) {
        for list in REGISTRATION_LISTS {
            let _removed = registry.remove_from_list(&self.identity, list);
        }
        self.logger.spend.batch.shutdown().await;
    }
}
