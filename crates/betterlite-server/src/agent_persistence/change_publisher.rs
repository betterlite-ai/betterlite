use std::future::Future;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_redis::config_sync::publish::PublishChange;

use super::AgentChangePublisher;

pub const AGENTS_OBJECT_TYPE: &str = "litellm_agentstable";

pub struct ConfigSyncAgentPublisher {
    pub publish: PublishChange<TableError>,
}

impl AgentChangePublisher for ConfigSyncAgentPublisher {
    fn agents_changed(&self) -> impl Future<Output = ()> + Send {
        let publish = Arc::clone(&self.publish);
        async move {
            if let Err(error) = publish(AGENTS_OBJECT_TYPE).await {
                tracing::warn!("agent config sync publish failed: {error}");
            }
        }
    }
}

mod tests;
