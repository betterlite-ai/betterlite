use std::collections::BTreeMap;
use std::path::PathBuf;

use betterlite_db::migration_deploy::DeployError;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "betterlite-db-admin",
    version,
    about = "Run betterlite database administration operations"
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Path to the betterlite configuration file"
    )]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub operation: Operation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Subcommand)]
pub enum Operation {
    #[command(about = "Run the database migration job")]
    Migrate,
}

impl Operation {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Migrate => "migrate",
        }
    }

    #[must_use]
    pub fn command(self, environment: &BTreeMap<String, String>) -> DbAdminCommand {
        match self {
            Self::Migrate => DbAdminCommand::Migrate(MigrationOptions {
                use_migrate: !str_to_bool(
                    environment.get("USE_PRISMA_DB_PUSH").map(String::as_str),
                ),
                use_v2_resolver: str_to_bool(Some(
                    environment
                        .get("USE_V2_MIGRATION_RESOLVER")
                        .map_or("true", String::as_str),
                )),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbAdminCommand {
    Migrate(MigrationOptions),
}

impl DbAdminCommand {
    #[must_use]
    pub const fn operation(self) -> Operation {
        match self {
            Self::Migrate(_) => Operation::Migrate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationOptions {
    pub use_migrate: bool,
    pub use_v2_resolver: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbAdminOutcome {
    Completed,
    RetriesExhausted,
    IndexJobFailed,
}

#[derive(Debug, thiserror::Error)]
pub enum DbAdminError {
    #[error("no native service is registered for the {operation} operation")]
    Unregistered { operation: &'static str },
    #[error("{source}")]
    Unrecoverable {
        #[source]
        source: MigrationError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error(transparent)]
    Deploy(#[from] DeployError),
}

// LiteLLM's extras str_to_bool: case-insensitive true/1/t/y/yes only, no stripping, no "on"
fn str_to_bool(value: Option<&str>) -> bool {
    let Some(text) = value else {
        return false;
    };
    matches!(
        text.to_lowercase().as_str(),
        "true" | "1" | "t" | "y" | "yes"
    )
}
