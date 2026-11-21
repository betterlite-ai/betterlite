use axum::Router;
use axum::routing::get;

use super::proxy::{PluginState, plugin_proxy};
use super::routes::{list_plugins, plugin_auth_token};

pub fn router(state: PluginState) -> Router {
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/auth-token", get(plugin_auth_token))
        // the wildcard does not match a bare trailing slash, so the empty path has its own route
        .route(
            "/plugin-proxy/{plugin_name}/",
            get(plugin_proxy)
                .post(plugin_proxy)
                .put(plugin_proxy)
                .delete(plugin_proxy)
                .patch(plugin_proxy)
                .options(plugin_proxy)
                .head(plugin_proxy),
        )
        .route(
            "/plugin-proxy/{plugin_name}/{*path}",
            get(plugin_proxy)
                .post(plugin_proxy)
                .put(plugin_proxy)
                .delete(plugin_proxy)
                .patch(plugin_proxy)
                .options(plugin_proxy)
                .head(plugin_proxy),
        )
        .with_state(state)
}

mod tests;
