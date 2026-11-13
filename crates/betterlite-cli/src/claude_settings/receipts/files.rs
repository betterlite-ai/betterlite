use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};

use super::error::ReceiptsError;

pub type FileFuture<'operation, Output> =
    Pin<Box<dyn Future<Output = Result<Output, ReceiptsError>> + Send + 'operation>>;

pub enum LandChange<'input> {
    Commit {
        staged: &'input StagedFile,
        path: &'input Path,
    },
    Remove {
        path: &'input Path,
    },
}

pub trait SettingsLanding: Send + Sync {
    fn land<'operation>(
        &'operation self,
        change: LandChange<'operation>,
    ) -> FileFuture<'operation, ()>;
}

impl SettingsLanding for PrivateFiles {
    fn land<'operation>(
        &'operation self,
        change: LandChange<'operation>,
    ) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            match change {
                LandChange::Commit { staged, path } => Self::commit_staged_json(self, staged, path)
                    .await
                    .map_err(ReceiptsError::from),
                LandChange::Remove { path } => {
                    let staged = StagedFile {
                        path: path.to_owned(),
                    };
                    Self::discard_staged_json(self, &staged)
                        .await
                        .map_err(ReceiptsError::from)
                }
            }
        })
    }
}
