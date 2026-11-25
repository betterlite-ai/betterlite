use async_trait::async_trait;

use crate::provider_attribution::ModelGroupLimits;

// DualCache's two calls this module makes: a set read that answers None for a missing key, and a set add
#[async_trait]
pub trait MinuteSetCache: Send + Sync {
    async fn async_get_cache(&self, key: &str) -> Result<Option<Vec<String>>, CacheReadError>;

    async fn async_set_cache_sadd(
        &self,
        key: &str,
        values: &[String],
        ttl: i64,
    ) -> Result<(), CacheWriteError>;
}

// a set read that failed: LiteLLM's check_available_usage catches the cache's exception and fails open
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("the cache could not read the minute set")]
pub struct CacheReadError;

// a set add that failed: LiteLLM logs the cache's exception and re-raises it into the task that fired the write
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("the cache could not add the key to the minute set")]
pub struct CacheWriteError;

// the Router reads check_available_usage makes, each answering what LiteLLM reads or the failure it catches
#[async_trait]
pub trait DynamicLimitRouter: Send + Sync {
    async fn get_model_group_info(
        &self,
        model_group: &str,
    ) -> Result<Option<ModelGroupLimits>, RouterReadError>;

    async fn get_model_group_usage(
        &self,
        model_group: &str,
    ) -> Result<(Option<i64>, Option<i64>), RouterReadError>;

    fn get_model_info(&self, id: &str) -> Option<String>;
}

// which router read failed; the handler logs it and fails open, so the variant names the read and nothing more
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RouterReadError {
    #[error("the model group info read failed")]
    GroupInfo,
    #[error("the model group usage read failed")]
    GroupUsage,
}
