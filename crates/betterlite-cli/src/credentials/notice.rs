use std::path::Path;

use crate::credential_lifecycle::outcome::SecretSave;
use crate::keyring::{DISABLE_KEYRING_ENV_VAR, KeyringUnusable, Read, SecretVault};

pub const KEYRING_INSTALL_HINT: &str = "pip install 'litellm[cli]'";

pub const KEYRING_ENABLE_HINT: &str = "keyring --enable (or unset PYTHON_KEYRING_BACKEND)";

pub const STRANDED_CREDENTIAL_MESSAGE: &str =
    "Logged out locally, but your credential is still in the OS keychain and could not be removed.";

pub const UNCHECKED_KEYCHAIN_MESSAGE: &str = "Logged out locally, but your OS keychain could not be checked, so a credential stored there by an earlier login may still be usable.";

pub type MessageSink<'sink> = &'sink (dyn Fn(&str) + Send + Sync);

#[must_use]
pub fn storage_notice(outcome: &SecretSave, token_path: &Path) -> String {
    let path = token_path.display();
    match outcome {
        SecretSave::SecretStored => "Credential stored in your OS keychain.".to_owned(),
        SecretSave::KeyringNotInstalled => format!(
            "Credential stored in {path} (owner-only). For OS keychain storage, install the keyring package with: {KEYRING_INSTALL_HINT}"
        ),
        SecretSave::KeyringDisabled => format!(
            "Keychain storage is off ({DISABLE_KEYRING_ENV_VAR}). Credential stored in {path} (owner-only)."
        ),
        SecretSave::KeyringUnreachable => {
            format!("No OS keychain available. Credential stored in {path} (owner-only).")
        }
        SecretSave::KeyringDiscardsWrites => format!(
            "Your keyring backend keeps nothing it is given, so the credential was stored in {path} (owner-only) instead. For OS keychain storage, run: {KEYRING_ENABLE_HINT}"
        ),
        SecretSave::CredentialNotSaved(not_saved) => format!(
            "Signed in, but the credential could not be saved to {path}: {}. Any login you already had is untouched. Run 'lite login' again once that path is writable, or 'lite logout' to clear whatever is stored now.",
            not_saved.source
        ),
        SecretSave::CredentialNotRecorded(_) => format!(
            "Signed in, and the credential is in your OS keychain, but {path} could not be replaced, so it still describes your previous login and may still hold its credential. Run 'lite login' again once that path is writable, or 'lite logout' to clear both."
        ),
    }
}

#[must_use]
pub fn keychain_unreadable_notice(vault: &dyn SecretVault) -> String {
    match vault.read() {
        Err(KeyringUnusable::KeyringNotInstalled) => format!(
            "Your credential is in your OS keychain, which this install cannot read without the keyring package. Install it with: {KEYRING_INSTALL_HINT}, or run 'lite login' to start over."
        ),
        Err(KeyringUnusable::KeyringDisabled) => format!(
            "Your credential is in your OS keychain, which {DISABLE_KEYRING_ENV_VAR} is blocking. Unset it, or run 'lite login' to start over."
        ),
        Err(KeyringUnusable::KeyringUnreachable) => {
            "Your credential is in your OS keychain, which could not be read. Unlock it, or run 'lite login' to start over.".to_owned()
        }
        Ok(Read::SecretFound(_) | Read::SecretMissing) => {
            "Your credential could not be read from your OS keychain. Run 'lite login' to start over."
                .to_owned()
        }
    }
}
