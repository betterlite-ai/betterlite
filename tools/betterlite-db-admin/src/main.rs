use std::env;
use std::ffi::OsString;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use betterlite_db::spend_backfills::BackfillPlan;
use betterlite_db_admin::command_shell::{
    AdminEnvironment, DbAdminBackend, DbAdminService, Diagnostics as _, Streams, run,
};
use betterlite_db_admin::commands::{Cli, Operation};
use clap::Parser as _;
use tokio::runtime::Builder;

use crate::database_views::{DatabaseViewsService, OFFLINE_TIMEOUT};
use crate::spend_backfills::SpendBackfillsService;

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
    backend
}

pub(crate) fn backend_for(arguments: &[OsString]) -> DbAdminBackend {
    let Ok(cli) = Cli::try_parse_from(arguments.iter().cloned()) else {
        return backend();
    };
    match cli.operation {
        Operation::DailyToolSpend { cutover } => {
            backend_with_plan(BackfillPlan::DailyToolSpend { cutover })
        }
        Operation::KeyTotalSpend => backend_with_plan(BackfillPlan::KeyTotalSpend),
        Operation::KeyTotalSpendFromLogs => backend_with_plan(BackfillPlan::KeyTotalSpendFromLogs),
        Operation::Migrate | Operation::DatabaseViews => backend(),
    }
}

fn backend_with_plan(plan: BackfillPlan) -> DbAdminBackend {
    let mut backend = backend();
    let backfills: Arc<dyn DbAdminService> = Arc::new(SpendBackfillsService { plan });
    backend.services.insert(Operation::DatabaseViews, backfills);
    backend
}

fn main() -> ExitCode {
    let arguments: Vec<OsString> = env::args_os().collect();
    let backend = backend_for(&arguments);
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
    let status = runtime.block_on(run(arguments, environment, &backend, &mut streams));
    ExitCode::from(status.code())
}
