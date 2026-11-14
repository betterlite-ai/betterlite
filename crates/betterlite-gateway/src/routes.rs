use betterlite_server::server::filter::ComponentRoutes;

pub const GATEWAY_ROUTES: ComponentRoutes = ComponentRoutes {
    prefixes: betterlite_routes::GATEWAY_PATH_PREFIXES,
    exact: &betterlite_routes::GATEWAY_EXACT_PATHS,
    mounts: &betterlite_routes::GATEWAY_MOUNT_PATHS,
};
