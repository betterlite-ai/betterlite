use std::io;

use thiserror::Error;

use crate::keyring::{Erase, KeyringUnusable, SecretErase, SecretWrite, Write};
use crate::token_records::error::TokenRecordError;

#[derive(Debug, Error)]
#[error("no store kept the credential: {source}")]
pub struct CredentialNotSaved {
    #[source]
    pub source: TokenRecordError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CredentialNotRecorded;

#[derive(Debug, Error)]
#[error("the token file could not be removed or rewritten: {source}")]
pub struct CredentialNotCleared {
    #[source]
    pub source: io::Error,
}

#[derive(Debug)]
pub enum SecretSave {
    SecretStored,
    KeyringDiscardsWrites,
    KeyringDisabled,
    KeyringNotInstalled,
    KeyringUnreachable,
    CredentialNotSaved(CredentialNotSaved),
    CredentialNotRecorded(CredentialNotRecorded),
}

#[derive(Debug)]
pub enum SecretClear {
    SecretErased,
    SecretStranded,
    KeyringDisabled,
    KeyringNotInstalled,
    KeyringUnreachable,
    CredentialNotCleared(CredentialNotCleared),
}

pub(super) const fn saved_as(outcome: &SecretWrite) -> SecretSave {
    match outcome {
        Ok(Write::SecretStored) => SecretSave::SecretStored,
        Ok(Write::KeyringDiscardsWrites) => SecretSave::KeyringDiscardsWrites,
        Err(unusable) => unusable_saved(*unusable),
    }
}

const fn unusable_saved(unusable: KeyringUnusable) -> SecretSave {
    match unusable {
        KeyringUnusable::KeyringDisabled => SecretSave::KeyringDisabled,
        KeyringUnusable::KeyringNotInstalled => SecretSave::KeyringNotInstalled,
        KeyringUnusable::KeyringUnreachable => SecretSave::KeyringUnreachable,
    }
}

pub(super) const fn cleared_as(outcome: &SecretErase) -> SecretClear {
    match outcome {
        Ok(Erase::SecretErased) => SecretClear::SecretErased,
        Ok(Erase::SecretStranded) => SecretClear::SecretStranded,
        Err(unusable) => unusable_cleared(*unusable),
    }
}

const fn unusable_cleared(unusable: KeyringUnusable) -> SecretClear {
    match unusable {
        KeyringUnusable::KeyringDisabled => SecretClear::KeyringDisabled,
        KeyringUnusable::KeyringNotInstalled => SecretClear::KeyringNotInstalled,
        KeyringUnusable::KeyringUnreachable => SecretClear::KeyringUnreachable,
    }
}
