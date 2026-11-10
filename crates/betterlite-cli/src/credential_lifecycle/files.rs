use std::io;
use std::path::PathBuf;

use async_trait::async_trait;
use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};
use tokio::fs;

use crate::token_records::commit::commit_token_file;
use crate::token_records::error::TokenRecordError;
use crate::token_records::file::read_token_file;
use crate::token_records::overwrite::overwrite_token_file_secret;
use crate::token_records::record::CliTokenRecord;
use crate::token_records::stage::stage_token_file;
use crate::token_records::write::write_token_file;

#[async_trait]
pub trait CredentialFiles: Send + Sync {
    fn read(&self) -> Option<CliTokenRecord>;

    async fn stage(&self, record: &CliTokenRecord) -> Result<StagedFile, TokenRecordError>;

    async fn commit(&self, staged: &StagedFile) -> bool;

    async fn discard(&self, staged: &StagedFile);

    async fn write(&self, record: &CliTokenRecord) -> Result<(), TokenRecordError>;

    async fn overwrite_scrubbed(&self, record: &CliTokenRecord) -> bool;

    async fn remove(&self) -> Result<(), io::Error>;
}

pub struct ManagedTokenFile {
    pub files: PrivateFiles,
    pub path: PathBuf,
}

#[async_trait]
impl CredentialFiles for ManagedTokenFile {
    fn read(&self) -> Option<CliTokenRecord> {
        read_token_file(&self.path)
    }

    async fn stage(&self, record: &CliTokenRecord) -> Result<StagedFile, TokenRecordError> {
        stage_token_file(&self.files, &self.path, record).await
    }

    async fn commit(&self, staged: &StagedFile) -> bool {
        commit_token_file(&self.files, staged, &self.path)
            .await
            .is_ok()
    }

    async fn discard(&self, staged: &StagedFile) {
        // LiteLLM lets a refused discard escape save_cli_token; a stray staged file is not worth that
        let _unused = self.files.discard_staged_json(staged).await;
    }

    async fn write(&self, record: &CliTokenRecord) -> Result<(), TokenRecordError> {
        write_token_file(&self.files, &self.path, record).await
    }

    async fn overwrite_scrubbed(&self, record: &CliTokenRecord) -> bool {
        overwrite_token_file_secret(&self.files, &self.path, record)
            .await
            .is_ok()
    }

    async fn remove(&self) -> Result<(), io::Error> {
        match fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(source),
        }
    }
}
