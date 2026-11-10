use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};

use super::error::ReceiptsError;

pub type FileFuture<'operation, Output> =
    Pin<Box<dyn Future<Output = Result<Output, ReceiptsError>> + Send + 'operation>>;

pub trait SettingsCommit: Send + Sync {
    fn commit<'operation>(
        &'operation self,
        staged: &'operation StagedFile,
        path: &'operation Path,
    ) -> FileFuture<'operation, ()>;
}

impl SettingsCommit for PrivateFiles {
    fn commit<'operation>(
        &'operation self,
        staged: &'operation StagedFile,
        path: &'operation Path,
    ) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            Self::commit_staged_json(self, staged, path)
                .await
                .map_err(ReceiptsError::from)
        })
    }
}
