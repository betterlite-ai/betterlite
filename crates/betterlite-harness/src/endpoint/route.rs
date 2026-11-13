pub const ROUTE_MESSAGES: &str = "messages";
pub const ROUTE_CHAT: &str = "chat/completions";
pub const ROUTE_RESPONSES: &str = "responses";
pub const POST_ROUTES: [&str; 3] = [ROUTE_MESSAGES, ROUTE_CHAT, ROUTE_RESPONSES];
pub const ROUTE_PREFIXES: [&str; 2] = ["", "/v1"];

#[must_use]
pub fn route_of(path: &str) -> &str {
    let stripped = path.trim_matches('/');
    match stripped.strip_prefix("v1/") {
        Some(route) => route,
        None => stripped,
    }
}
