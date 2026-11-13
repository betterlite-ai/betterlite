pub mod cli;

use betterlite_gateway::serve::ComponentRoutes;
use betterlite_gateway::serve::serve_component;
use cli::Cli;
use std::process::ExitCode;

pub const BACKEND_ROUTES: ComponentRoutes = ComponentRoutes {
    prefixes: betterlite_routes::BACKEND_PATH_PREFIXES,
    exact: &betterlite_routes::BACKEND_EXACT_PATHS,
    mounts: &betterlite_routes::BACKEND_MOUNT_PATHS,
};

pub fn run(cli: Cli) -> ExitCode {
    serve_component(cli.config, cli.host, cli.port, BACKEND_ROUTES)
}
