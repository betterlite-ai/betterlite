pub mod cli;
pub mod routes;
pub mod serve;

use cli::Cli;
use std::process::ExitCode;

pub fn run(cli: Cli) -> ExitCode {
    serve::serve_component(cli.config, cli.host, cli.port, routes::GATEWAY_ROUTES)
}
