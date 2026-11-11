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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FailedStep {
    pub name: Option<String>,
    pub sqlstate: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DeployOutcome {
    Complete,
    TimedOut,
    SchemaNotBaselined,
    UnfinishedHistory { identity: Option<FailedIdentity> },
    FailedStep(FailedStep),
    Deadlock { detail: String },
    Contention { detail: String },
    Failed { detail: String },
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
