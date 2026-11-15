use betterlite_gateway::cli::Cli;
use betterlite_gateway::routes::GATEWAY_ROUTES;
use betterlite_server::server::component::serve_component;
use clap::Parser as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    serve_component(&cli.config, &cli.host, cli.port, GATEWAY_ROUTES)
}
