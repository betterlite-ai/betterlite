#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Messages,
    Chat,
    Responses,
}

impl Route {
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::Chat => "chat/completions",
            Self::Responses => "responses",
        }
    }
}

pub const POST_ROUTES: [Route; 3] = [Route::Messages, Route::Chat, Route::Responses];
pub const ROUTE_PREFIXES: [&str; 2] = ["", "/v1"];

#[must_use]
pub fn route_of(path: &str) -> Option<Route> {
    let stripped = path.trim_matches('/');
    match stripped.strip_prefix("v1/").unwrap_or(stripped) {
        "messages" => Some(Route::Messages),
        "chat/completions" => Some(Route::Chat),
        "responses" => Some(Route::Responses),
        _ => None,
    }
}
