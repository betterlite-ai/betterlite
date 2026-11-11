use crate::migration_ledger::budget::MAX_MIGRATE_DEPLOY_ATTEMPTS;

use super::outcome::{DeployFailure, FailedIdentity, FailedStep, database_detail, sqlstate_of};

const UNVERIFIED_LEAD: &str =
    "Migration completion could not be verified. LiteLLM startup has stopped.";
pub(super) const MANUAL_LEAD: &str =
    "Database migration failed and cannot be auto-recovered. Manual intervention required.";
const PERMISSION_LEAD: &str = "Database migration failed due to insufficient permissions. Please grant the required privileges and retry.";

pub(super) fn unverified_completion(reported: Option<&FailedIdentity>, reason: &str) -> String {
    let history = reported.map_or_else(
        || reason.to_owned(),
        |found| format!("{} started at {}", found.name, found.started_at),
    );
    format!(
        "{UNVERIFIED_LEAD}\n\nMigration history (migration name and start time):\n{history}\n\n\
A migration has a start record but no successful completion record. LiteLLM cannot determine \
whether its SQL committed from this record alone. Startup stopped to avoid repeating or skipping \
database changes.\n\n\
Before resolving, stop other migration runners and inspect _prisma_migrations, the named \
migration.sql from this build, database logs, and the actual database objects and data. Use the \
same database and this build's schema and migration files for recovery:\n\
- Only after verifying every migration change is present, run prisma migrate resolve --applied \
<migration_name>, then retry startup.\n\
- Only after verifying no migration changes remain (or fully undoing partial changes), run prisma \
migrate resolve --rolled-back <migration_name>, then retry startup. This command updates history; \
it does not undo SQL.\n\
Replace <migration_name> with the reported name. If the outcome remains uncertain, leave migration \
history unchanged and contact your database administrator. Repeated restarts alone will not \
resolve this state."
    )
}

pub(super) fn insufficient_privileges(step: &FailedStep) -> String {
    format!(
        "{PERMISSION_LEAD}\n\n{}",
        describe(Some(&step.name), step.sqlstate().as_deref(), &step.detail())
    )
}

pub(super) fn manual_for_step(step: &FailedStep) -> String {
    format!(
        "{MANUAL_LEAD}\n\n{}",
        describe(Some(&step.name), step.sqlstate().as_deref(), &step.detail())
    )
}

pub(super) fn manual_for_failure(failure: &DeployFailure) -> String {
    let detail = match failure {
        DeployFailure::Database(source) => describe(
            None,
            sqlstate_of(source).as_deref(),
            &database_detail(source),
        ),
        DeployFailure::MigrationLog {
            name,
            source,
            ledger,
        } => describe(
            None,
            sqlstate_of(source).as_deref(),
            &format!(
                "migration {name} failed ({}) and recording the failure in the migration ledger also failed: {ledger}",
                database_detail(source)
            ),
        ),
        DeployFailure::InvalidSqlLog {
            name,
            source,
            ledger,
        } => describe(
            None,
            None,
            &format!(
                "migration {name} has SQL that is not UTF-8 ({source}) and recording the failure also failed: {ledger}"
            ),
        ),
    };
    format!("{MANUAL_LEAD}\n\n{detail}")
}

pub(super) fn attempts_exhausted() -> String {
    format!(
        "Database migration failed after {MAX_MIGRATE_DEPLOY_ATTEMPTS} attempts that made no \
progress (timeouts or deadlock retries). Check database connectivity, load, and \
_prisma_migrations ledger state, and raise LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT if the attempts \
timed out."
    )
}

pub(super) fn describe(name: Option<&str>, sqlstate: Option<&str>, detail: &str) -> String {
    let mut rendered = String::new();
    if let Some(value) = name {
        rendered.push_str("Migration: ");
        rendered.push_str(value);
        rendered.push('\n');
    }
    if let Some(code) = sqlstate {
        rendered.push_str("SQLSTATE: ");
        rendered.push_str(code);
        rendered.push('\n');
    }
    rendered.push_str("Database error:\n");
    rendered.push_str(detail);
    rendered
}
