use async_trait::async_trait;

use super::extraction::{ArgillaItem, StandardLoggingPayload};

#[async_trait]
pub trait DatasetHook: Send + Sync {
    async fn async_dataset_hook(
        &self,
        item: ArgillaItem,
        payload: Option<&StandardLoggingPayload>,
    ) -> Result<Option<ArgillaItem>, DatasetHookError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DatasetHookError {
    #[error("the callback does not implement async_dataset_hook")]
    NotImplemented,
    #[error("the callback's dataset hook failed")]
    Failed,
}
