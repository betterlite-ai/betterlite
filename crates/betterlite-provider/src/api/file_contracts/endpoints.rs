use std::error::Error;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_openai::files::records::{
    BinaryContent, FileCreate, FileDeleted, FileObject, FilePage,
};

use super::config::FileParams;

pub struct FileListRequest<'request> {
    pub purpose: Option<&'request str>,
    pub user_api_key_dict: &'request UserAPIKeyAuth,
    pub limit: Option<i64>,
    pub after: Option<&'request str>,
    pub data: &'request FileParams,
}

#[async_trait]
pub trait BaseFileEndpoints: Send + Sync {
    type Error: Error + Send + Sync;

    async fn acreate_file(
        &self,
        create_file_request: &FileCreate,
        target_model_names: &[String],
        user_api_key_dict: &UserAPIKeyAuth,
    ) -> Result<FileObject, Self::Error>;

    async fn afile_retrieve(&self, file_id: &str) -> Result<FileObject, Self::Error>;

    async fn afile_list(&self, request: FileListRequest<'_>) -> Result<FilePage, Self::Error>;

    async fn afile_delete(
        &self,
        file_id: &str,
        data: &FileParams,
    ) -> Result<FileDeleted, Self::Error>;

    async fn afile_content(
        &self,
        file_id: &str,
        data: &FileParams,
    ) -> Result<BinaryContent, Self::Error>;
}
