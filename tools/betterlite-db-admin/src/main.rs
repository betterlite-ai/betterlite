use std::env;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use betterlite_db_admin::command_shell::{
    AdminEnvironment, DbAdminBackend, Diagnostics as _, Streams, run,
};
use betterlite_db_admin::commands::Operation;
use tokio::runtime::Builder;

use crate::database_views::DatabaseViewsService;

mod database_repair;
mod database_startup;
mod database_views;
mod spend_backfills;
mod spend_partitions;

fn main() -> ExitCode {
    let mut backend = DbAdminBackend::default();
    backend
        .services
        .insert(Operation::DatabaseViews, Arc::new(DatabaseViewsService));
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
