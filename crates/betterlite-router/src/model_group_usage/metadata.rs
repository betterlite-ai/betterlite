use std::error::Error;

use async_trait::async_trait;

use crate::model_group_info::group::ModelGroupInfo;

// Router._cached_get_model_group_info behind B10047's GroupInfoCache: the LRU, its capacity and its invalidation stay there.
#[async_trait]
pub trait GroupMetadata: Send + Sync {
    type Error: Error + Send + 'static;

    async fn cached_group_info(
        &self,
        model_group: &str,
    ) -> Result<Option<ModelGroupInfo>, Self::Error>;
}
