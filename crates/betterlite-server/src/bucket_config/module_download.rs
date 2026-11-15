use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::str::{Utf8Error, from_utf8};
use std::sync::Arc;

use tokio::fs;

use super::gcs_reader::GcsBucketOpener;
use super::object::BucketObjectReader;
use super::s3_reader::{S3Get, S3Reader};
use super::source::BucketType;

#[derive(Debug, thiserror::Error)]
pub enum ModuleDownloadError {
    #[error("the module object {object_key} could not be read from the bucket")]
    Read { object_key: String },
    #[error("the module object {object_key} is not valid UTF-8")]
    Decode {
        object_key: String,
        #[source]
        source: Utf8Error,
    },
    #[error("the directory for the module file {path} could not be created")]
    Directories {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the module file {path} could not be written")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub struct BucketModuleDownloader {
    pub environment: BTreeMap<String, String>,
    pub s3: Arc<dyn S3Get>,
    pub gcs: Arc<dyn GcsBucketOpener>,
}

impl BucketModuleDownloader {
    // LiteLLM builds a fresh client and fresh credentials for every module download
    pub async fn download_module(
        &self,
        source_type: BucketType,
        bucket_name: &str,
        object_key: &str,
        local_file_path: &Path,
    ) -> Result<(), ModuleDownloadError> {
        let reader: Box<dyn BucketObjectReader> = match source_type {
            BucketType::S3 => Box::new(
                S3Reader::initialize(bucket_name, &self.environment, Arc::clone(&self.s3)).await,
            ),
            BucketType::Gcs => self.gcs.open(bucket_name),
        };
        let Some(bytes) = reader.read_object(object_key).await else {
            return Err(ModuleDownloadError::Read {
                object_key: object_key.to_owned(),
            });
        };
        let text = match from_utf8(&bytes) {
            Ok(text) => text,
            Err(source) => {
                return Err(ModuleDownloadError::Decode {
                    object_key: object_key.to_owned(),
                    source,
                });
            }
        };
        write_module_file(local_file_path, text).await?;
        tracing::debug!(
            object_key,
            path = %local_file_path.display(),
            "Python file downloaded"
        );
        Ok(())
    }
}

async fn write_module_file(local_file_path: &Path, text: &str) -> Result<(), ModuleDownloadError> {
    let parent = local_file_path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty());
    if let Some(directory) = parent {
        fs::create_dir_all(directory)
            .await
            .map_err(|source| ModuleDownloadError::Directories {
                path: directory.to_path_buf(),
                source,
            })?;
    }
    // LiteLLM logs the file text here; a module can hold secrets, so only the path is logged
    fs::write(local_file_path, text)
        .await
        .map_err(|source| ModuleDownloadError::Write {
            path: local_file_path.to_path_buf(),
            source,
        })
}

mod tests;
