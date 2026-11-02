use betterlite_auth::pass_through_registry::{PassThroughMetadata, PassThroughRouteType};

pub fn remove_endpoint_routes(
    registrations: &mut Vec<(String, PassThroughMetadata)>,
    openai_routes: &mut Vec<String>,
    endpoint_id: &str,
) {
    let mut remaining = Vec::new();
    for (key, metadata) in registrations.drain(..) {
        if metadata.endpoint_id == endpoint_id {
            remove_first(openai_routes, &metadata.path);
            if metadata.route_type == PassThroughRouteType::Subpath {
                let wildcard = format!("{}/*", metadata.path.trim_end_matches('/'));
                remove_first(openai_routes, &wildcard);
            }
        } else {
            remaining.push((key, metadata));
        }
    }
    *registrations = remaining;
}

fn remove_first(list: &mut Vec<String>, value: &str) {
    if let Some(position) = list.iter().position(|entry| entry == value) {
        list.remove(position);
    }
}

mod tests;
