pub const HOT_ROUTE_PATHS: [&str; 5] = [
    "/health/liveliness",
    "/health/liveness",
    "/v1/chat/completions",
    "/chat/completions",
    "/v1/messages",
];

// Starlette matches in registration order, so LiteLLM puts its busiest paths first; axum matches by path, so this order is only an audit
pub fn hot_routes_first<'path, Routes>(routes: Routes) -> Vec<&'path str>
where
    Routes: IntoIterator<Item = &'path str>,
{
    let mut ordered: Vec<&str> = routes.into_iter().collect();
    ordered.sort_by_key(|path| !HOT_ROUTE_PATHS.contains(path));
    ordered
}
