use std::collections::BTreeMap;
use std::path::PathBuf;

use betterlite_db::database_repair::RepairError;
use betterlite_db::migration_deploy::DeployError;
use betterlite_db::migration_ledger::connection::ConnectionError;
use betterlite_db::spend_backfills::BackfillError;
use betterlite_db::spend_partitions::{SpendPartitionError, SpendPartitionOperation};
use chrono::{DateTime, Utc};
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Subcommand)]
pub enum Operation {
    #[command(about = "Run the database migration job")]
    Migrate,
    #[command(about = "Create the views LiteLLM's proxy reads, as the offline db_scripts job does")]
    DatabaseViews,
    #[command(
        about = "Backfill daily tool spend from spend logs before a cutover timestamp",
        long_about = "Backfill LiteLLM_DailyToolSpend from LiteLLM_SpendLogToolIndex and LiteLLM_SpendLogs.\n\nPre-deploy index rows may include tools that were declared but never invoked, and each such request's full spend is attributed to every indexed tool. Coverage is bounded by spend-log retention. Existing (date, tool_name) buckets are skipped, never merged."
    )]
    DailyToolSpend {
        #[arg(
            long,
            value_name = "TIMESTAMP",
            help = "UTC cutover; index rows before it are backfilled"
        )]
        cutover: DateTime<Utc>,
    },
    #[command(about = "Raise each active and archived key's total_spend to its current spend")]
    KeyTotalSpend,
    #[command(
        about = "Rebuild resetting keys' lifetime spend from spend logs",
        long_about = "Rebuild resetting keys' total_spend from LiteLLM_SpendLogs.\n\nRun key-total-spend first; this is its optional follow-up. Coverage is bounded by spend-log retention. Run it while the proxy is idle or with traffic paused, because queued spend-log batches would be missing from the rebuilt sum."
    )]
    KeyTotalSpendFromLogs,
    #[command(
        about = "Convert LiteLLM_SpendLogs into a startTime range-partitioned table offline",
        long_about = "Convert LiteLLM_SpendLogs into a startTime range-partitioned table, following db_scripts/partition_spend_logs.sql.\n\nRun it with the proxy idle or traffic paused, and test on a staging copy with a backup first. Postgres cannot convert a populated table in place: the old rows stay in LiteLLM_SpendLogs_legacy, no rows are copied, and the new table starts empty. Afterwards enable use_spend_logs_partitioning and a maximum_spend_logs_retention_period separately in general_settings, and keep SPEND_LOG_PARTITION_INTERVAL consistent with the partitions you create later. To roll back, run unpartition-spend-logs and disable use_spend_logs_partitioning."
    )]
    PartitionSpendLogs,
    #[command(
        about = "Convert a partitioned LiteLLM_SpendLogs back to the plain table offline",
        long_about = "Convert a partitioned LiteLLM_SpendLogs back into the plain table of the default LiteLLM schema, following db_scripts/unpartition_spend_logs.sql.\n\nRun it during a low-traffic window and take a backup first: every surviving row is copied, and duplicate request_id values at different startTime values collapse to one arbitrary row. Afterwards disable use_spend_logs_partitioning, or remove it, so retention returns to DELETE-based cleanup."
    )]
    UnpartitionSpendLogs,
    #[command(about = "Fill unassigned spend logs' team_id from their verification token")]
    UpdateUnassignedTeams,
    #[command(about = "Import verification tokens from the LiteLLM CSV export")]
    MigrateVerificationTokens {
        #[arg(
            long,
            value_name = "PATH",
            help = "Path to the verification-token CSV export"
        )]
        csv: PathBuf,
    },
}

impl Operation {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Migrate => "migrate",
            Self::DatabaseViews => "database-views",
            Self::DailyToolSpend { .. } => "daily-tool-spend",
            Self::KeyTotalSpend => "key-total-spend",
            Self::KeyTotalSpendFromLogs => "key-total-spend-from-logs",
            Self::PartitionSpendLogs => "partition-spend-logs",
            Self::UnpartitionSpendLogs => "unpartition-spend-logs",
            Self::UpdateUnassignedTeams => "update-unassigned-teams",
            Self::MigrateVerificationTokens { .. } => "migrate-verification-tokens",
        }
    }

    const fn of_spend_partition(operation: SpendPartitionOperation) -> Self {
        match operation {
            SpendPartitionOperation::Partition => Self::PartitionSpendLogs,
            SpendPartitionOperation::Unpartition => Self::UnpartitionSpendLogs,
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
            Self::DailyToolSpend { cutover } => {
                DbAdminCommand::DailyToolSpend(DailyToolSpendOptions { cutover })
            }
            Self::KeyTotalSpend => DbAdminCommand::KeyTotalSpend,
            Self::KeyTotalSpendFromLogs => DbAdminCommand::KeyTotalSpendFromLogs,
            Self::PartitionSpendLogs => {
                DbAdminCommand::SpendPartition(SpendPartitionOperation::Partition)
            }
            Self::UnpartitionSpendLogs => {
                DbAdminCommand::SpendPartition(SpendPartitionOperation::Unpartition)
            }
            Self::UpdateUnassignedTeams => DbAdminCommand::UpdateUnassignedTeams,
            Self::MigrateVerificationTokens { csv } => {
                DbAdminCommand::MigrateVerificationTokens(csv)
            }
        }
    }
}

// the registry keys a service by operation, so the parsed cutover is not part of the lookup
pub const DAILY_TOOL_SPEND_KEY: Operation = Operation::DailyToolSpend {
    cutover: DateTime::<Utc>::UNIX_EPOCH,
};

pub const VERIFICATION_TOKENS_KEY: Operation = Operation::MigrateVerificationTokens {
    csv: PathBuf::new(),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyToolSpendOptions {
    pub cutover: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbAdminCommand {
    Migrate(MigrationOptions),
    DatabaseViews,
    DailyToolSpend(DailyToolSpendOptions),
    KeyTotalSpend,
    KeyTotalSpendFromLogs,
    SpendPartition(SpendPartitionOperation),
    UpdateUnassignedTeams,
    MigrateVerificationTokens(PathBuf),
}

impl DbAdminCommand {
    #[must_use]
    pub const fn operation(&self) -> Operation {
        match self {
            Self::Migrate(_) => Operation::Migrate,
            Self::DatabaseViews => Operation::DatabaseViews,
            Self::DailyToolSpend(_) => DAILY_TOOL_SPEND_KEY,
            Self::KeyTotalSpend => Operation::KeyTotalSpend,
            Self::KeyTotalSpendFromLogs => Operation::KeyTotalSpendFromLogs,
            Self::SpendPartition(operation) => Operation::of_spend_partition(*operation),
            Self::UpdateUnassignedTeams => Operation::UpdateUnassignedTeams,
            Self::MigrateVerificationTokens(_) => Operation::MigrateVerificationTokens {
                csv: PathBuf::new(),
            },
        }
    }
}

// padding keeps the larger daily options variant within three times this variant's size
#[repr(align(8))]
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
    #[error("{source}")]
    SpendBackfill {
        #[source]
        source: SpendBackfillError,
    },
    #[error("{source}")]
    SpendPartition {
        #[source]
        source: SpendPartitionAdminError,
    },
    #[error(transparent)]
    DatabaseRepair(#[from] RepairError),
}

impl From<BackfillError> for DbAdminError {
    fn from(source: BackfillError) -> Self {
        Self::SpendBackfill {
            source: SpendBackfillError::Backfill { source },
        }
    }
}

impl From<SpendPartitionError> for DbAdminError {
    fn from(source: SpendPartitionError) -> Self {
        Self::SpendPartition {
            source: SpendPartitionAdminError::Conversion { source },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SpendPartitionAdminError {
    #[error("the partition database connection could not be opened: {source}")]
    Connection {
        #[source]
        source: ConnectionError,
    },
    #[error("{source}")]
    Conversion {
        #[source]
        source: SpendPartitionError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SpendBackfillError {
    #[error("the backfill database connection could not be opened: {source}")]
    Connection {
        #[source]
        source: ConnectionError,
    },
    #[error("{source}")]
    Backfill {
        #[source]
        source: BackfillError,
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
    value.is_some_and(|text| ["true", "1", "t", "y", "yes"].contains(&text.to_lowercase().as_str()))
}
