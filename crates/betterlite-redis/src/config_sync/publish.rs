use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;

use super::channel::config_sync_channel;
use super::error::ConfigSyncError;
use super::params::ResyncAppliedParam;
use crate::redis_cache::RedisCache;

pub type PublishFuture<F> = Pin<Box<dyn Future<Output = Result<(), F>> + Send>>;
pub type PublishChange<F> = Arc<dyn Fn(&str) -> PublishFuture<F> + Send + Sync>;

#[derive(Serialize)]
struct ConfigChangeMessage {
    object_type: String,
}

pub async fn publish_config_change(redis_cache: Option<&RedisCache>, object_type: &str) {
    let Some(cache) = redis_cache else {
        return;
    };
    if let Err(error) = publish_over_transport(cache, object_type).await {
        tracing::warn!("config sync publish for {object_type} failed: {error}");
    }
}

pub async fn publish_config_param_change(redis_cache: Option<&RedisCache>, param_name: &str) {
    let applied = ResyncAppliedParam::ALL
        .iter()
        .any(|param| param.name() == param_name);
    if !applied {
        tracing::debug!(
            "config sync publish for {param_name} skipped: no resync callback applies this param outside proxy startup"
        );
        return;
    }
    publish_config_change(redis_cache, param_name).await;
}

async fn publish_over_transport(
    redis_cache: &RedisCache,
    object_type: &str,
) -> Result<(), ConfigSyncError> {
    let message = serde_json::to_string(&ConfigChangeMessage {
        object_type: object_type.to_owned(),
    })?;
    let channel = config_sync_channel(redis_cache.namespace.as_deref());
    let client = redis_cache.transport.pubsub_client().await?;
    client.publish(&channel, &message).await?;
    Ok(())
}

#[must_use]
pub fn redis_config_change_publisher<F>(redis_cache: Option<Arc<RedisCache>>) -> PublishChange<F> {
    Arc::new(move |object_type: &str| {
        let cache = redis_cache.clone();
        let name = object_type.to_owned();
        Box::pin(async move {
            publish_config_change(cache.as_deref(), &name).await;
            Ok(())
        })
    })
}
