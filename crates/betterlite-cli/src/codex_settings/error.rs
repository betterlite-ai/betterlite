use std::io;
use std::path::PathBuf;
use std::string::FromUtf8Error;

use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;
use toml_edit::TomlError;

use super::receipt::ReceiptSource;

#[derive(Debug, Error)]
pub enum CodexSettingsError {
    #[error("cannot read the Codex settings at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the Codex settings at {path} are not valid UTF-8")]
    NotUtf8 {
        path: PathBuf,
        #[source]
        source: FromUtf8Error,
    },
    #[error("the Codex settings at {path} are not valid TOML; no settings were changed")]
    Parse {
        path: PathBuf,
        #[source]
        source: TomlError,
    },
    #[error("cannot resolve the Codex settings path {path}: {source}")]
    Resolve {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the Codex settings path is not valid UTF-8, so it cannot be recorded in a receipt")]
    NonUtf8Path { path: PathBuf },
    #[error("Codex model_providers must be a TOML table; no settings were changed")]
    ProvidersNotTable,
    #[error("Codex model_providers.litellm must be a TOML table; no settings were changed")]
    ProviderNotTable,
    #[error("cannot read the Codex configure receipt at {path}; no settings were changed")]
    Receipt {
        path: PathBuf,
        #[source]
        source: ReceiptSource,
    },
    #[error(
        "Codex 0.129.0 or newer (stable) is required before saving a gateway key, because older versions allow repository settings to redirect authenticated requests; install or update Codex, check `codex --version`, then retry"
    )]
    UnsafeCodex,
    #[error("cannot stage the Codex configure receipt at {path}: {source}")]
    StageReceipt {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot stage the Codex settings at {path}; no settings were changed: {source}")]
    StageSettings {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error(
        "cannot write the Codex configure receipt at {path}; no settings were changed: {source}"
    )]
    WriteReceipt {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error(
        "cannot write the Codex settings at {path}; the earlier receipt was restored: {source}"
    )]
    WriteSettings {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error(
        "the Codex settings were not written and the Codex configure receipt at {path} could not be restored: {source}"
    )]
    RestoreReceipt {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error("Codex is not configured by `lite configure codex`; nothing to undo")]
    NothingToUndo,
    #[error(
        "cannot finish undoing the Codex configuration; the receipt was kept for retry: {source}"
    )]
    Undo {
        #[source]
        source: PrivateJsonError,
    },
}
