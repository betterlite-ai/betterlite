use std::str::Utf8Error;

use tokio_postgres::Error as PostgresError;

use super::StepCause;

pub(super) const DEADLOCK_MARKER: &str = "deadlock detected";
pub(super) const DEADLOCK_SQLSTATE: &str = "40P01";
pub(super) const CANCELED_SQLSTATE: &str = "57014";
const PERMISSION_SQLSTATE: &str = "42501";

const PERMISSION_MARKERS: [&str; 5] = [
    "database error code: 42501",
    "must be owner of table",
    "permission denied for schema",
    "permission denied for table",
    "must be owner of schema",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FailedIdentity {
    pub name: String,
    pub started_at: String,
}

#[derive(Debug)]
pub(super) struct FailedStep {
    pub name: String,
    pub source: StepSource,
}

impl FailedStep {
    #[must_use]
    pub(super) fn sqlstate(&self) -> Option<String> {
        match &self.source {
            StepSource::Database(source) => sqlstate_of(source),
            StepSource::Utf8(_) => None,
        }
    }

    #[must_use]
    pub(super) fn detail(&self) -> String {
        match &self.source {
            StepSource::Database(source) => database_detail(source),
            StepSource::Utf8(source) => source.to_string(),
        }
    }
}

#[derive(Debug)]
pub(super) enum StepSource {
    Database(PostgresError),
    Utf8(Utf8Error),
}

impl StepSource {
    #[must_use]
    pub(super) fn into_cause(self) -> StepCause {
        match self {
            Self::Database(source) => StepCause::Database(source),
            Self::Utf8(source) => StepCause::Utf8(source),
        }
    }
}

#[derive(Debug)]
pub(super) enum DeployOutcome {
    Complete,
    TimedOut,
    SchemaNotBaselined,
    UnfinishedHistory { identity: Option<FailedIdentity> },
    FailedStep(FailedStep),
    Deadlock,
    Contention { detail: String },
    Failed(DeployFailure),
}

#[derive(Debug)]
pub(super) enum DeployFailure {
    Database(PostgresError),
    MigrationLog {
        name: String,
        source: PostgresError,
        ledger: PostgresError,
    },
    InvalidSqlLog {
        name: String,
        source: Utf8Error,
        ledger: PostgresError,
    },
}

impl DeployFailure {
    // For the dual failures the primary error enters the cause chain; the message keeps both
    #[must_use]
    pub(super) fn into_cause(self) -> StepCause {
        match self {
            Self::Database(source) | Self::MigrationLog { source, .. } => {
                StepCause::Database(source)
            }
            Self::InvalidSqlLog { source, .. } => StepCause::Utf8(source),
        }
    }
}

// Mirrors _is_permission_error: the five case-insensitive patterns, plus the native SQLSTATE for them
#[must_use]
pub(super) fn is_permission_error(sqlstate: Option<&str>, detail: &str) -> bool {
    if sqlstate == Some(PERMISSION_SQLSTATE) {
        return true;
    }
    let lowered = detail.to_ascii_lowercase();
    PERMISSION_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

// The source marker is case-sensitive; a native deadlock also arrives as SQLSTATE 40P01
#[must_use]
pub(super) fn is_deadlock(sqlstate: Option<&str>, detail: &str) -> bool {
    if sqlstate == Some(DEADLOCK_SQLSTATE) {
        return true;
    }
    detail.contains(DEADLOCK_MARKER)
}

pub(super) fn sqlstate_of(source: &PostgresError) -> Option<String> {
    source
        .as_db_error()
        .map(|error| error.code().code().to_owned())
}

// The DbError Display carries the severity, message, detail and hint that Error's "db error" leaves out
pub(super) fn database_detail(source: &PostgresError) -> String {
    source
        .as_db_error()
        .map_or_else(|| source.to_string(), ToString::to_string)
}
