use betterlite_backend::BACKEND_ROUTES;
use betterlite_backend::cli::Cli;
use betterlite_server::server::component::serve_component;
use clap::Parser as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    serve_component(&cli.config, &cli.host, cli.port, BACKEND_ROUTES)
}
