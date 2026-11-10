use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cli::keyring::{
    Erase, KeyringUnusable, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;

pub(super) type Events = Mutex<Vec<&'static str>>;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VaultInput {
    #[serde(default)]
    blob: Option<String>,
    #[serde(default)]
    refuse: Vec<VaultRefusal>,
    #[serde(default)]
    failure: Option<FailureKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VaultRefusal {
    Unavailable,
    Unwritable,
    Unerasable,
    Discards,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FailureKind {
    Disabled,
    NotInstalled,
    Unreachable,
}

impl FailureKind {
    const fn unusable(self) -> KeyringUnusable {
        match self {
            Self::Disabled => KeyringUnusable::KeyringDisabled,
            Self::NotInstalled => KeyringUnusable::KeyringNotInstalled,
            Self::Unreachable => KeyringUnusable::KeyringUnreachable,
        }
    }
}

pub(super) struct FakeVault {
    pub(super) blob: Option<SecretString>,
    failure: KeyringUnusable,
    refusals: Vec<VaultRefusal>,
    pub(super) events: Arc<Events>,
}

impl FakeVault {
    pub(super) fn new(input: &VaultInput, events: Arc<Events>) -> Self {
        Self {
            blob: input.blob.as_deref().map(SecretString::from),
            failure: input.failure.unwrap_or(FailureKind::Unreachable).unusable(),
            refusals: input.refuse.clone(),
            events,
        }
    }

    pub(super) fn value(&self) -> Value {
        self.blob.as_ref().map_or(Value::Null, |blob| {
            serde_json::from_str(blob.expose_secret()).unwrap_or(Value::Null)
        })
    }

    fn note(&self, event: &'static str) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

impl SecretVault for FakeVault {
    fn read(&self) -> SecretRead {
        self.note("vault_read");
        if self.refusals.contains(&VaultRefusal::Unavailable) {
            return Err(self.failure);
        }
        Ok(self
            .blob
            .as_ref()
            .map_or(Read::SecretMissing, |blob| Read::SecretFound(blob.clone())))
    }

    fn write(&mut self, blob: &SecretString) -> SecretWrite {
        self.note("vault_write");
        if self.refusals.contains(&VaultRefusal::Unavailable)
            || self.refusals.contains(&VaultRefusal::Unwritable)
        {
            return Err(self.failure);
        }
        if self.refusals.contains(&VaultRefusal::Discards) {
            return Ok(Write::KeyringDiscardsWrites);
        }
        self.blob = Some(blob.clone());
        Ok(Write::SecretStored)
    }

    fn erase(&mut self) -> SecretErase {
        self.note("vault_erase");
        if self.refusals.contains(&VaultRefusal::Unavailable) {
            return Err(self.failure);
        }
        if self.refusals.contains(&VaultRefusal::Unerasable) {
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
