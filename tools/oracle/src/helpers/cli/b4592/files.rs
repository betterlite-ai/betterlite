use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use async_trait::async_trait;
use betterlite_cli::credential_lifecycle::files::{CredentialFiles, ManagedTokenFile};
use betterlite_cli::token_records::error::TokenRecordError;
use betterlite_cli::token_records::record::CliTokenRecord;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};
use serde::Deserialize;

use super::vault::Events;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Failures {
    #[serde(default)]
    pub(super) stage: Option<StageFailure>,
    #[serde(default)]
    pub(super) refuse: Vec<Refusal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Refusal {
    Commit,
    Write,
    Overwrite,
    Remove,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StageFailure {
    Always,
    WhilePresent,
}

pub(super) struct ScriptedFiles {
    pub(super) inner: ManagedTokenFile,
    stage: Option<StageFailure>,
    refusals: Vec<Refusal>,
    events: Arc<Events>,
}

impl ScriptedFiles {
    pub(super) fn new(path: &Path, failures: &Failures, events: Arc<Events>) -> Self {
        Self {
            inner: ManagedTokenFile {
                files: PrivateFiles::default(),
                path: PathBuf::from(path),
            },
            stage: failures.stage,
            refusals: failures.refuse.clone(),
            events,
        }
    }

    fn note(&self, event: &'static str) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn refusal(&self, reason: &'static str) -> TokenRecordError {
        TokenRecordError::Stage {
            source: PrivateJsonError::Stage(self.inner.path.clone(), io::Error::other(reason)),
        }
    }

    fn write_refusal(&self) -> TokenRecordError {
        TokenRecordError::Write {
            source: PrivateJsonError::Commit(
                self.inner.path.clone(),
                io::Error::other("scripted write failure"),
            ),
        }
    }
}

#[async_trait]
impl CredentialFiles for ScriptedFiles {
    fn read(&self) -> Option<CliTokenRecord> {
        self.note("read");
        self.inner.read()
    }

    async fn stage(&self, record: &CliTokenRecord) -> Result<StagedFile, TokenRecordError> {
        self.note("stage");
        let failing = match self.stage {
            Some(StageFailure::Always) => true,
            Some(StageFailure::WhilePresent) => self.inner.path.exists(),
            None => false,
        };
        if failing {
            return Err(self.refusal("scripted stage failure"));
        }
        self.inner.stage(record).await
    }

    async fn commit(&self, staged: &StagedFile) -> bool {
        self.note("commit");
        if self.refusals.contains(&Refusal::Commit) {
            return false;
        }
        self.inner.commit(staged).await
    }

    async fn discard(&self, staged: &StagedFile) {
        self.note("discard");
        self.inner.discard(staged).await;
    }

    async fn write(&self, record: &CliTokenRecord) -> Result<(), TokenRecordError> {
        self.note("write");
        if self.refusals.contains(&Refusal::Write) {
            return Err(self.write_refusal());
        }
        self.inner.write(record).await
    }

    async fn overwrite_scrubbed(&self, record: &CliTokenRecord) -> bool {
        self.note("overwrite");
        if self.refusals.contains(&Refusal::Overwrite) {
            return false;
        }
        self.inner.overwrite_scrubbed(record).await
    }

    async fn remove(&self) -> Result<(), io::Error> {
        self.note("remove");
        if self.refusals.contains(&Refusal::Remove) {
            return Err(io::Error::other("scripted removal failure"));
        }
        self.inner.remove().await
    }
}
