use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles, StagedFile};

use super::error::ReceiptsError;

pub type FileFuture<'operation, Output> =
    Pin<Box<dyn Future<Output = Result<Output, ReceiptsError>> + Send + 'operation>>;

pub trait SettingsFiles: Send + Sync {
    fn ensure_dir<'operation>(
        &'operation self,
        directory: &'operation Path,
    ) -> FileFuture<'operation, ()>;

    fn stage<'operation>(
        &'operation self,
        path: &'operation Path,
        document: &'operation JsonValue,
    ) -> FileFuture<'operation, StagedFile>;

    fn commit<'operation>(
        &'operation self,
        staged: &'operation StagedFile,
        path: &'operation Path,
    ) -> FileFuture<'operation, ()>;

    fn discard<'operation>(
        &'operation self,
        staged: &'operation StagedFile,
    ) -> FileFuture<'operation, ()>;

    fn remove<'operation>(&'operation self, path: &'operation Path) -> FileFuture<'operation, ()>;
}

impl SettingsFiles for PrivateFiles {
    fn ensure_dir<'operation>(
        &'operation self,
        directory: &'operation Path,
    ) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            Self::ensure_private_dir(self, directory)
                .await
                .map_err(|source| ReceiptsError::Write {
                    path: directory.to_owned(),
                    source,
                })
        })
    }

    fn stage<'operation>(
        &'operation self,
        path: &'operation Path,
        document: &'operation JsonValue,
    ) -> FileFuture<'operation, StagedFile> {
        Box::pin(async move {
            let staged =
                PrivateDocument::json(document).map_err(|source| ReceiptsError::Write {
                    path: path.to_owned(),
                    source,
                })?;
            Self::stage_private(self, path, staged)
                .await
                .map_err(|source| ReceiptsError::Write {
                    path: path.to_owned(),
                    source,
                })
        })
    }

    fn commit<'operation>(
        &'operation self,
        staged: &'operation StagedFile,
        path: &'operation Path,
    ) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            Self::commit_staged_json(self, staged, path)
                .await
                .map_err(|source| ReceiptsError::Write {
                    path: path.to_owned(),
                    source,
                })
        })
    }

    fn discard<'operation>(
        &'operation self,
        staged: &'operation StagedFile,
    ) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            Self::discard_staged_json(self, staged)
                .await
                .map_err(|source| ReceiptsError::Discard {
                    path: staged.path.clone(),
                    source,
                })
        })
    }

    fn remove<'operation>(&'operation self, path: &'operation Path) -> FileFuture<'operation, ()> {
        let staged = StagedFile {
            path: path.to_owned(),
        };
        Box::pin(async move {
            Self::discard_staged_json(self, &staged)
                .await
                .map_err(|source| ReceiptsError::Remove {
                    path: path.to_owned(),
                    source,
                })
        })
    }
}
