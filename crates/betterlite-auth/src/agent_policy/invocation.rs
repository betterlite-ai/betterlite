use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[must_use]
pub fn invocation_target(route: &str, body: &IndexMap<String, Json>) -> Option<String> {
    let components: Vec<&str> = route.trim_matches('/').split('/').collect();
    let path = match components.first() {
        Some(&"v1") => components.get(1..).unwrap_or_default(),
        _ => components.as_slice(),
    };
    if let [collection, target, ..] = path
        && *collection == "a2a"
    {
        return if target.is_empty() {
            None
        } else {
            Some((*target).to_owned())
        };
    }
    let model = match body.get("model") {
        Some(Json::Text(model)) => model,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => return None,
    };
    model
        .strip_prefix("a2a/")
        .filter(|agent| !agent.is_empty())
        .map(str::to_owned)
}
