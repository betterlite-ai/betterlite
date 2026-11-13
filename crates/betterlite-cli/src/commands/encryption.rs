use std::io::Write;

use betterlite_redact::secret::Redactor;
use clap::{Args, Subcommand};

use crate::cli_context::CliContext;

use super::keys::api::ApiError;
use super::{Client, ClientError};

mod migrate;

#[derive(Debug, Subcommand)]
#[command(about = "Migrate at-rest credentials to AES-256-GCM and attest residual state")]
pub enum EncryptionCommand {
    Migrate(MigrateEncryption),
}

#[derive(Debug, Args)]
pub struct MigrateEncryption {
    #[arg(
        long,
        help = "Read-only residual scan (no writes). Reports legacy values remaining."
    )]
    check: bool,
    #[arg(
        long = "dry-run",
        help = "Run the full migration walkers without writing any changes."
    )]
    dry_run: bool,
}

pub struct EncryptionPorts<'run> {
    pub context: &'run CliContext,
    pub client: &'run Client,
    pub redactor: &'run Redactor,
    pub stdout: &'run mut dyn Write,
    pub stderr: &'run mut dyn Write,
}

#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("cannot encode the JSON text")]
    Json(#[source] serde_json::Error),
    #[error("the response from {url} is not a JSON object")]
    NotAnObject { url: String },
    #[error("the response field {field} is not valid: {source}")]
    ReplyField {
        field: String,
        #[source]
        source: serde_json::Error,
    },
}

pub fn run_encryption_command(
    command: EncryptionCommand,
    ports: EncryptionPorts<'_>,
) -> Result<(), EncryptionError> {
    match command {
        EncryptionCommand::Migrate(options) => migrate::migrate_encryption(&options, ports),
    }
}

mod tests;
