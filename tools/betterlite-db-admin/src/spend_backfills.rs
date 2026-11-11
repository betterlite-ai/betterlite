use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::spend_backfills::{BackfillOutcome, BackfillPlan, run_plan};
use betterlite_db_admin::command_shell::{DbAdminFuture, DbAdminService, ResolvedConfig};
use betterlite_db_admin::commands::{
    DbAdminCommand, DbAdminError, DbAdminOutcome, SpendBackfillError,
};
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::ExposeSecret as _;
use tokio_postgres::NoTls;

pub(crate) struct SpendBackfillsService;

impl DbAdminService for SpendBackfillsService {
    fn run(&self, command: DbAdminCommand, config: ResolvedConfig) -> DbAdminFuture<'_> {
        Box::pin(async move {
            let session = session(&config).await?;
            let plan = plan(command)?;
            let outcome = run_plan(&session, plan).await?;
            Ok(DbAdminOutcome::Views {
                lines: vec![outcome_line(outcome)],
            })
        })
    }
}

async fn session(config: &ResolvedConfig) -> Result<Client, DbAdminError> {
    let configuration: tokio_postgres::Config = config
        .database_url
        .expose_secret()
        .parse()
        .map_err(|source| DbAdminError::SpendBackfill {
            source: SpendBackfillError::Url { source },
        })?;
    let manager = Manager::new(configuration, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .map_err(|source| DbAdminError::SpendBackfill {
            source: SpendBackfillError::Pool { source },
        })?;
    let store = PostgresStore { pool: Some(pool) };
    store
        .session()
        .await
        .map_err(|source| DbAdminError::SpendBackfill {
            source: SpendBackfillError::Session { source },
        })
}

const fn plan(command: DbAdminCommand) -> Result<BackfillPlan, DbAdminError> {
    match command {
        DbAdminCommand::DailyToolSpend(options) => Ok(BackfillPlan::DailyToolSpend {
            cutover: options.cutover,
        }),
        DbAdminCommand::KeyTotalSpend => Ok(BackfillPlan::KeyTotalSpend),
        DbAdminCommand::KeyTotalSpendFromLogs => Ok(BackfillPlan::KeyTotalSpendFromLogs),
        other @ (DbAdminCommand::Migrate(_) | DbAdminCommand::DatabaseViews) => {
            Err(DbAdminError::Unregistered {
                operation: other.operation().name(),
            })
        }
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
