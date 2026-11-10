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
    #[command(about = "Create the views LiteLLM's proxy reads, as the offline db_scripts job does")]
    DatabaseViews,
}

impl Operation {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Migrate => "migrate",
            Self::DatabaseViews => "database-views",
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
            Self::DatabaseViews => DbAdminCommand::DatabaseViews,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbAdminCommand {
    Migrate(MigrationOptions),
    DatabaseViews,
}

impl DbAdminCommand {
    #[must_use]
    pub const fn operation(self) -> Operation {
        match self {
            Self::Migrate(_) => Operation::Migrate,
            Self::DatabaseViews => Operation::DatabaseViews,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationOptions {
    pub use_migrate: bool,
    pub use_v2_resolver: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbAdminOutcome {
    Completed,
    RetriesExhausted,
    IndexJobFailed,
    Views { lines: Vec<String> },
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
    #[error("{source}")]
    Views {
        lines: Vec<String>,
        #[source]
        source: ViewsError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ViewsError {
    #[error("the database URL is not a PostgreSQL connection string: {source}")]
    Url {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("could not connect to the database: {source}")]
    Connect {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("the database did not answer within {seconds} seconds")]
    Timeout { seconds: u64 },
    #[error("a view statement failed: {source}{}", .source.as_db_error().map_or_else(String::new, |server| format!(": {server}")))]
    Statement {
        #[source]
        source: tokio_postgres::Error,
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
