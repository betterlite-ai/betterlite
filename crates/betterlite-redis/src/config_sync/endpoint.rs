use std::future::Future;
use std::time::Duration;

use super::error::ConfigSyncError;

pub struct ConfigSyncMessage {
    pub channel: String,
    pub payload: Vec<u8>,
}

pub trait ConfigSyncPubSubClient: Send + Sync {
    type Subscription: ConfigSyncSubscription;

    fn subscribe(&self)
    -> impl Future<Output = Result<Self::Subscription, ConfigSyncError>> + Send;
}

pub trait ConfigSyncSubscription: Send {
    fn next_message(
        &mut self,
        timeout: Duration,
    ) -> impl Future<Output = Result<Option<ConfigSyncMessage>, ConfigSyncError>> + Send;

    fn close(&mut self) -> impl Future<Output = ()> + Send;
}
