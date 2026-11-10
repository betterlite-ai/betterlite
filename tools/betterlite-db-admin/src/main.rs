use std::collections::BTreeMap;
use std::env;
use std::io::{self, Write as _};
use std::process::ExitCode;

use betterlite_db_admin::command_shell::{AdminEnvironment, DbAdminBackend, Streams, run};
use tokio::runtime::Builder;

fn main() -> ExitCode {
    let environment = process_environment();
    let backend = DbAdminBackend::default();
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            drop(writeln!(
                io::stderr(),
                "the async runtime could not be built: {error}"
            ));
            return ExitCode::FAILURE;
        }
    };
    let mut output = io::stdout();
    let mut errors = io::stderr();
    let mut streams = Streams {
        output: &mut output,
        errors: &mut errors,
    };
    let status = runtime.block_on(run(env::args_os(), environment, &backend, &mut streams));
    ExitCode::from(status.code())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the binary is where the process environment enters, and it hands every variable to the config loader"
)]
fn process_environment() -> AdminEnvironment {
    let mut variables = BTreeMap::new();
    for (raw_name, raw_value) in env::vars_os() {
        let (Ok(name), Ok(value)) = (raw_name.into_string(), raw_value.into_string()) else {
            drop(writeln!(
                io::stderr(),
                "an environment variable is not UTF-8 and is ignored"
            ));
            continue;
        };
        variables.insert(name, value);
    }
    AdminEnvironment {
        variables,
        home: env::home_dir(),
    }
}
