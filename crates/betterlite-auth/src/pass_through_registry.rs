mod tests;

use crate::auxiliary_routes::MAPPED_PASS_THROUGH_ROUTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassThroughRouteType {
    Exact,
    Subpath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassThroughMetadata {
    pub endpoint_id: String,
    pub route_type: PassThroughRouteType,
    pub path: String,
    pub methods: Vec<String>,
    pub auth: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct PassThroughRegistry {
    root_path: String,
    registrations: Vec<(String, PassThroughMetadata)>,
}

impl PassThroughRegistry {
    #[must_use]
    pub fn new(root_path: String, registrations: Vec<(String, PassThroughMetadata)>) -> Self {
        Self {
            root_path,
            registrations,
        }
    }

    #[must_use]
    pub fn route_for_registry_lookup(&self, route: &str) -> String {
        self.normalize_route(route)
            .unwrap_or_else(|| route.to_owned())
    }

    #[must_use]
    pub fn is_registered_pass_through_route(&self, route: &str) -> bool {
        if let Some(normalized) = self.normalize_route(route) {
            if MAPPED_PASS_THROUGH_ROUTES
                .iter()
                .any(|prefix| normalized.starts_with(prefix))
            {
                return true;
            }
        }
        let comparison_route = self.route_for_registry_lookup(route);
        self.registrations
            .iter()
            .any(|(key, _)| key_matches(key, &comparison_route))
    }

    #[must_use]
    pub fn get_registered_pass_through_route(
        &self,
        route: &str,
        method: Option<&str>,
    ) -> Option<&PassThroughMetadata> {
        let comparison_route = self.route_for_registry_lookup(route);
        self.registrations.iter().find_map(|(key, metadata)| {
            let parts = key.splitn(4, ':').collect::<Vec<_>>();
            let (route_type, registered_path) = parts.get(1).zip(parts.get(2))?;
            if !matches_path(route_type, registered_path, &comparison_route)
                || !method_allowed(parts.get(3).copied(), metadata, method)
            {
                return None;
            }
            Some(metadata)
        })
    }

    fn normalize_route(&self, route: &str) -> Option<String> {
        if self.root_path.is_empty() || self.root_path == "/" {
            return Some(route.to_owned());
        }
        let prefix = format!("{}/", self.root_path);
        route
            .starts_with(&prefix)
            .then(|| route[self.root_path.len()..].to_owned())
    }
}

fn key_matches(key: &str, route: &str) -> bool {
    let parts = key.splitn(4, ':').collect::<Vec<_>>();
    let Some((route_type, registered_path)) = parts.get(1).zip(parts.get(2)) else {
        return false;
    };
    matches_path(route_type, registered_path, route)
}

fn matches_path(route_type: &str, registered_path: &str, route: &str) -> bool {
    match route_type {
        "exact" => route == registered_path,
        "subpath" => route == registered_path || route.starts_with(&format!("{registered_path}/")),
        _ => false,
    }
}

fn method_allowed(
    key_methods: Option<&str>,
    metadata: &PassThroughMetadata,
    method: Option<&str>,
) -> bool {
    if method.is_none() {
        return true;
    }
    if !metadata.methods.is_empty() {
        return metadata
            .methods
            .iter()
            .any(|value| Some(value.as_str()) == method);
    }
    key_methods.is_none()
        || key_methods.is_some_and(|value| {
            value
                .split(',')
                .any(|key_method| Some(key_method) == method)
        })
}
