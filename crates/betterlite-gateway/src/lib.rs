pub mod cli;
pub mod routes;

use betterlite_server::server::component::serve_component;
use cli::Cli;
use std::process::ExitCode;

pub fn run(cli: Cli) -> ExitCode {
    serve_component(cli.config, cli.host, cli.port, routes::GATEWAY_ROUTES)
}
