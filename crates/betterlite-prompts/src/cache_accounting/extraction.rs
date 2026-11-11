use crate::arguments::json::Json;

// The first header whose name matches user-agent case-insensitively; a value that is not text is no user agent.
#[must_use]
pub fn request_user_agent(request_kwargs: &Json) -> Option<&str> {
    let headers = request_kwargs
        .as_object()?
        .get("proxy_server_request")?
        .as_object()?
        .get("headers")?
        .as_object()?;
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        .map(|(_, value)| value)
        .and_then(Json::as_str)
}

// The system parameter is a string, or the block list Anthropic's messages route takes; anything else is no system at all.
#[must_use]
pub fn request_system(request_kwargs: &Json) -> Option<&Json> {
    let system = request_kwargs.as_object()?.get("system")?;
    match system {
        Json::Text(_) | Json::List(_) => Some(system),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => None,
    }
}
