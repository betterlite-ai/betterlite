use std::env;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use betterlite_db_admin::command_shell::{
    AdminEnvironment, DbAdminBackend, DbAdminService, Diagnostics as _, Streams, run,
};
use betterlite_db_admin::commands::{DAILY_TOOL_SPEND_KEY, Operation, VERIFICATION_TOKENS_KEY};
use tokio::runtime::Builder;

use crate::database_repair::DatabaseRepairService;
use crate::database_views::{DatabaseViewsService, OFFLINE_TIMEOUT};
use crate::spend_backfills::SpendBackfillsService;
use crate::spend_partitions::SpendPartitionsService;

mod database_repair;
mod database_startup;
mod database_views;
mod spend_backfills;
mod spend_partitions;

fn backend() -> DbAdminBackend {
    let mut backend = DbAdminBackend::default();
    let views = DatabaseViewsService {
        deadline: OFFLINE_TIMEOUT,
    };
    backend
        .services
        .insert(Operation::DatabaseViews, Arc::new(views));
    let backfills: Arc<dyn DbAdminService> = Arc::new(SpendBackfillsService);
    backend
        .services
        .insert(DAILY_TOOL_SPEND_KEY, Arc::clone(&backfills));
    backend
        .services
        .insert(Operation::KeyTotalSpend, Arc::clone(&backfills));
    backend
        .services
        .insert(Operation::KeyTotalSpendFromLogs, backfills);
    let partitions: Arc<dyn DbAdminService> = Arc::new(SpendPartitionsService);
    backend
        .services
        .insert(Operation::PartitionSpendLogs, Arc::clone(&partitions));
    backend
        .services
        .insert(Operation::UnpartitionSpendLogs, partitions);
    let repair: Arc<dyn DbAdminService> = Arc::new(DatabaseRepairService);
    backend
        .services
        .insert(Operation::UpdateUnassignedTeams, Arc::clone(&repair));
    backend.services.insert(VERIFICATION_TOKENS_KEY, repair);
    backend
}

fn main() -> ExitCode {
    let backend = backend();
    let mut output = io::stdout();
    let mut errors = io::stderr();
    let mut streams = Streams {
        output: &mut output,
        errors: &mut errors,
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "the binary is where the process environment enters, and it hands every variable to the config loader"
    )]
    let process_variables = env::vars_os();
    let environment = AdminEnvironment::from_os(process_variables, env::home_dir(), &mut streams);
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            streams.error(&format!("the async runtime could not be built: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let status = runtime.block_on(run(env::args_os(), environment, &backend, &mut streams));
    ExitCode::from(status.code())
}
