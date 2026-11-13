pub mod cli;

use betterlite_server::server::component::serve_component;
use betterlite_server::server::filter::ComponentRoutes;
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
