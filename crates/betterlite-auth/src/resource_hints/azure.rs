#[must_use]
pub fn router_model_in_endpoint(endpoint: &str, router_models: &[String]) -> Option<String> {
    let parts: Vec<&str> = endpoint.split('/').collect();
    if parts.len() < 2 {
        return None;
    }
    parts
        .into_iter()
        .find(|part| router_models.iter().any(|model| model.as_str() == *part))
        .map(str::to_owned)
}
