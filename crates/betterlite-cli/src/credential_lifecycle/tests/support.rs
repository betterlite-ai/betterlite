use std::fs;
use std::io;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};
use secrecy::SecretString;
use tempfile::TempDir;

use crate::credential_lifecycle::files::{CredentialFiles, ManagedTokenFile};
use crate::keyring::{
    Erase, KeyringUnusable, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write,
};
use crate::token_records::error::TokenRecordError;
use crate::token_records::file::read_token_file;
use crate::token_records::record::CliTokenRecord;
use crate::token_records::secret::encode_secret;

pub(crate) const SERVER: &str = "https://gateway.example";
pub(crate) const OTHER_SERVER: &str = "https://other.example";

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn record(timestamp: f64) -> CliTokenRecord {
    CliTokenRecord {
        base_url: SERVER.to_owned(),
        key: Some(secret("sk-secret")),
        user_id: "user-1".to_owned(),
        user_email: "user@example.com".to_owned(),
        user_role: "cli".to_owned(),
        auth_header_name: "Authorization".to_owned(),
        jwt_token: secret("jwt-secret"),
        timestamp,
        expires_at: None,
        refresh_token: Some(secret("refresh-secret")),
        client_id: None,
        token_endpoint: None,
        revocation_endpoint: None,
        resource: None,
        team_id: None,
    }
}

pub(crate) fn encoded(record: &CliTokenRecord) -> SecretString {
    encode_secret(record).expect("encodes the stored secret")
}

pub(crate) fn metadata(timestamp: f64) -> CliTokenRecord {
    let mut record = record(timestamp);
    record.key = None;
    record.jwt_token = secret("");
    record.refresh_token = None;
    record
}

pub(crate) fn vault_entry(
    base_url: &str,
    key: &str,
    refresh: Option<&str>,
    timestamp: f64,
) -> SecretString {
    let mut record = metadata(timestamp);
    record.base_url = base_url.to_owned();
    record.key = Some(secret(key));
    record.refresh_token = refresh.map(secret);
    encoded(&record)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultRefusal {
    Unavailable,
    Unwritable,
    Unerasable,
    Discards,
}

#[derive(Default)]
pub(crate) struct Calls {
    pub(crate) reads: usize,
    pub(crate) writes: Vec<SecretString>,
    pub(crate) erases: usize,
}

impl Calls {
    fn read(&mut self) {
        self.reads = self.reads.saturating_add(1);
    }

    fn write(&mut self, blob: &SecretString) {
        self.writes.push(blob.clone());
    }

    fn erase(&mut self) {
        self.erases = self.erases.saturating_add(1);
    }
}

pub(crate) struct FakeVault {
    pub blob: Option<SecretString>,
    pub failure: KeyringUnusable,
    pub refusals: Vec<VaultRefusal>,
    pub calls: Mutex<Calls>,
}

pub(crate) fn fake_vault() -> FakeVault {
    FakeVault {
        blob: None,
        failure: KeyringUnusable::KeyringUnreachable,
        refusals: Vec::new(),
        calls: Mutex::new(Calls::default()),
    }
}

impl FakeVault {
    fn refuses(&self, refusal: VaultRefusal) -> bool {
        self.refusals.contains(&refusal)
    }

    pub(crate) fn reads(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reads
    }

    pub(crate) fn writes(&self) -> Vec<SecretString> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .writes
            .clone()
    }

    pub(crate) fn erases(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .erases
    }
}

impl SecretVault for FakeVault {
    fn read(&self) -> SecretRead {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .read();
        if self.refuses(VaultRefusal::Unavailable) {
            return Err(self.failure);
        }
        Ok(self
            .blob
            .as_ref()
            .map_or(Read::SecretMissing, |blob| Read::SecretFound(blob.clone())))
    }

    fn write(&mut self, blob: &SecretString) -> SecretWrite {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write(blob);
        if self.refuses(VaultRefusal::Unavailable) || self.refuses(VaultRefusal::Unwritable) {
            return Err(self.failure);
        }
        if self.refuses(VaultRefusal::Discards) {
            return Ok(Write::KeyringDiscardsWrites);
        }
        self.blob = Some(blob.clone());
        Ok(Write::SecretStored)
    }

    fn erase(&mut self) -> SecretErase {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .erase();
        if self.refuses(VaultRefusal::Unavailable) {
            return Err(self.failure);
        }
        if self.refuses(VaultRefusal::Unerasable) {
            return Ok(if self.blob.is_some() {
                Erase::SecretStranded
            } else {
                Erase::SecretErased
            });
        }
        self.blob = None;
        Ok(Erase::SecretErased)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    Commit,
    Write,
    Overwrite,
    Remove,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StageRefusal {
    #[default]
    Off,
    Always,
    WhilePresent,
}

pub(crate) struct ScriptedFiles {
    pub inner: ManagedTokenFile,
    pub stage: StageRefusal,
    pub refusals: Vec<Refusal>,
    pub _directory: TempDir,
}

pub(crate) fn scripted_files() -> ScriptedFiles {
    let directory = tempfile::tempdir().expect("makes a token directory");
    let path = directory.path().join("token.json");
    ScriptedFiles {
        inner: ManagedTokenFile {
            files: PrivateFiles::default(),
            path,
        },
        stage: StageRefusal::Off,
        refusals: Vec::new(),
        _directory: directory,
    }
}

impl ScriptedFiles {
    fn refuses(&self, refusal: Refusal) -> bool {
        self.refusals.contains(&refusal)
    }

    pub(crate) fn put(&self, record: &CliTokenRecord) {
        fs::write(
            &self.inner.path,
            serde_json::to_vec(record).expect("serializes the fixture"),
        )
        .expect("writes the fixture");
    }

    pub(crate) fn stored(&self) -> Option<CliTokenRecord> {
        read_token_file(&self.inner.path)
    }

    pub(crate) fn exists(&self) -> bool {
        self.inner.path.exists()
    }

    fn write_refusal(&self) -> TokenRecordError {
        TokenRecordError::Write {
            source: PrivateJsonError::Commit(
                self.inner.path.clone(),
                io::Error::other("the fixture refuses the write"),
            ),
        }
    }
}

#[async_trait]
impl CredentialFiles for ScriptedFiles {
    fn read(&self) -> Option<CliTokenRecord> {
        self.inner.read()
    }

    async fn stage(&self, record: &CliTokenRecord) -> Result<StagedFile, TokenRecordError> {
        let failing = match self.stage {
            StageRefusal::Off => false,
            StageRefusal::Always => true,
            StageRefusal::WhilePresent => self.inner.path.exists(),
        };
        if failing {
            return Err(TokenRecordError::Stage {
                source: PrivateJsonError::Stage(
                    self.inner.path.clone(),
                    io::Error::other("the fixture refuses the stage"),
                ),
            });
        }
        self.inner.stage(record).await
    }

    async fn commit(&self, staged: &StagedFile) -> bool {
        if self.refuses(Refusal::Commit) {
            return false;
        }
        self.inner.commit(staged).await
    }

    async fn discard(&self, staged: &StagedFile) {
        self.inner.discard(staged).await;
    }

    async fn write(&self, record: &CliTokenRecord) -> Result<(), TokenRecordError> {
        if self.refuses(Refusal::Write) {
            return Err(self.write_refusal());
        }
        self.inner.write(record).await
    }

    async fn overwrite_scrubbed(&self, record: &CliTokenRecord) -> bool {
        if self.refuses(Refusal::Overwrite) {
            return false;
        }
        self.inner.overwrite_scrubbed(record).await
    }

    async fn remove(&self) -> Result<(), io::Error> {
        if self.refuses(Refusal::Remove) {
            return Err(io::Error::other("the fixture refuses the removal"));
        }
        self.inner.remove().await
    }
}
