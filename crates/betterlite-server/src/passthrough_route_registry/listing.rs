use betterlite_auth::pass_through_registry::PassThroughMetadata;

#[must_use]
pub fn registered_route_keys(registrations: &[(String, PassThroughMetadata)]) -> Vec<String> {
    let mut keys = Vec::new();
    for (key, _) in registrations {
        keys.push(key.clone());
    }
    keys
}

mod tests;
