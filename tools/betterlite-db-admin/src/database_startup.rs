use betterlite_config::sections::database_startup::job::{
    JobOutcome, RequestLogIndexBuild, run_migration_job,
};
use betterlite_config::sections::database_startup::packaged;
use betterlite_config::sections::database_startup::startup::StartupSettings;
use betterlite_db::migration_ledger::lock::SystemJitter;
use betterlite_db::migration_recovery::RecoveryError;
use betterlite_db_admin::command_shell::{DbAdminFuture, DbAdminService, ResolvedConfig};
use betterlite_db_admin::commands::{DbAdminCommand, DbAdminError, DbAdminOutcome, MigrationError};

mod tests;

// migrations/run.py in native form: DATABASE_URL, the caller's USE_PRISMA_DB_PUSH and
// USE_V2_MIGRATION_RESOLVER switches, then the same migration job the proxy runs at startup.
pub(crate) struct DatabaseStartupService;

impl DbAdminService for DatabaseStartupService {
    fn run(&self, command: DbAdminCommand, config: ResolvedConfig) -> DbAdminFuture<'_> {
        Box::pin(async move {
            let operation = command.operation();
            let DbAdminCommand::Migrate(options) = command else {
                return Err(DbAdminError::Unregistered {
                    operation: <&'static str>::from(operation),
                });
            };
            let mut settings = StartupSettings::from_config(&config.settings);
            settings.database_url = Some(config.database_url);
            settings.use_prisma_db_push = !options.use_migrate;
            settings.use_v2_migration_resolver = options.use_v2_resolver;
            let jitter = SystemJitter::try_new()
                .map_err(|source| MigrationError::Recovery(RecoveryError::from(source)))?;
            let build = RequestLogIndexBuild {
                settings: &settings,
            };
            let outcome = run_migration_job(
                &settings,
                &packaged::migrations(),
                packaged::SCHEMA_PRISMA,
                &jitter,
                &build,
            )
            .await
            .map_err(MigrationError::Startup)?;
            Ok(match outcome {
                JobOutcome::Completed => DbAdminOutcome::Completed,
                JobOutcome::SetupFailed => DbAdminOutcome::RetriesExhausted,
                JobOutcome::IndexJobFailed => DbAdminOutcome::IndexJobFailed,
            })
        })
    }
}
