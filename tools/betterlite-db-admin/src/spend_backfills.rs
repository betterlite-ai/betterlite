use betterlite_db::migration_ledger::connection::connect;
use betterlite_db::spend_backfills::{BackfillOutcome, BackfillPlan, run_plan};
use betterlite_db_admin::command_shell::{DbAdminFuture, DbAdminService, ResolvedConfig};
use betterlite_db_admin::commands::{
    DbAdminCommand, DbAdminError, DbAdminOutcome, SpendBackfillError,
};

pub(crate) struct SpendBackfillsService;

impl DbAdminService for SpendBackfillsService {
    fn run(&self, command: DbAdminCommand, config: ResolvedConfig) -> DbAdminFuture<'_> {
        Box::pin(async move {
            let plan = plan(command)?;
            let session = connect(&config.database_url).await.map_err(|source| {
                DbAdminError::SpendBackfill {
                    source: SpendBackfillError::Connection { source },
                }
            })?;
            let outcome = run_plan(&*session, plan).await?;
            Ok(DbAdminOutcome::Views {
                lines: vec![outcome_line(outcome)],
            })
        })
    }
}

const fn plan(command: DbAdminCommand) -> Result<BackfillPlan, DbAdminError> {
    match command {
        DbAdminCommand::DailyToolSpend(options) => Ok(BackfillPlan::DailyToolSpend {
            cutover: options.cutover,
        }),
        DbAdminCommand::KeyTotalSpend => Ok(BackfillPlan::KeyTotalSpend),
        DbAdminCommand::KeyTotalSpendFromLogs => Ok(BackfillPlan::KeyTotalSpendFromLogs),
        other @ (DbAdminCommand::Migrate(_)
        | DbAdminCommand::DatabaseViews
        | DbAdminCommand::SpendPartition(_)) => Err(DbAdminError::Unregistered {
            operation: other.operation().name(),
        }),
    }
}

fn outcome_line(outcome: BackfillOutcome) -> String {
    match outcome {
        BackfillOutcome::DailyToolSpend { buckets } => {
            format!("daily-tool-spend inserted {buckets} buckets.")
        }
        BackfillOutcome::KeyTotalSpend {
            active_keys,
            archived_keys,
        } => format!(
            "key-total-spend updated {active_keys} active and {archived_keys} archived keys."
        ),
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys,
            archived_keys,
        } => format!(
            "key-total-spend-from-logs updated {active_keys} active and {archived_keys} archived keys."
        ),
    }
}

mod tests;
