pub mod cli;

use betterlite_server::server::filter::ComponentRoutes;

pub const BACKEND_ROUTES: ComponentRoutes = ComponentRoutes {
    prefixes: betterlite_routes::BACKEND_PATH_PREFIXES,
    exact: &betterlite_routes::BACKEND_EXACT_PATHS,
    mounts: &betterlite_routes::BACKEND_MOUNT_PATHS,
};
