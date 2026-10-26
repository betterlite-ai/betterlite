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
    pub root_path: String,
    pub registrations: Vec<(String, PassThroughMetadata)>,
}

impl PassThroughRegistry {
    #[must_use]
    pub fn is_registered(&self, route: &str) -> bool {
        if let Some(normalized) = self.normalize_route(route)
            && MAPPED_PASS_THROUGH_ROUTES
                .iter()
                .any(|prefix| normalized.starts_with(prefix))
        {
            return true;
        }
        let comparison_route = self
            .normalize_route(route)
            .unwrap_or_else(|| route.to_owned());
        !self.matching_registrations(&comparison_route).is_empty()
    }

    #[must_use]
    pub fn get_registered(
        &self,
        route: &str,
        method: Option<&str>,
    ) -> Option<&PassThroughMetadata> {
        let comparison_route = self
            .normalize_route(route)
            .unwrap_or_else(|| route.to_owned());
        self.matching_registrations(&comparison_route)
            .into_iter()
            .find_map(|(key, metadata)| {
                let parts = key.splitn(4, ':').collect::<Vec<_>>();
                method_allowed(parts.get(3).copied(), metadata, method).then_some(metadata)
            })
    }

    #[must_use]
    pub fn normalize_route(&self, route: &str) -> Option<String> {
        if self.root_path.is_empty() || self.root_path == "/" {
            return Some(route.to_owned());
        }
        let prefix = format!("{}/", self.root_path);
        route.starts_with(&prefix).then(|| {
            route
                .get(self.root_path.len()..)
                .map(str::to_owned)
                .unwrap_or_default()
        })
    }

    fn matching_registrations(&self, route: &str) -> Vec<(&str, &PassThroughMetadata)> {
        let mut matches = Vec::new();
        for (key, metadata) in &self.registrations {
            let parts = key.splitn(4, ':').collect::<Vec<_>>();
            if let Some((route_type, registered_path)) = parts.get(1).zip(parts.get(2))
                && matches_path(route_type, registered_path, route)
            {
                matches.push((key.as_str(), metadata));
            }
        }
        matches
    }
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
    key_methods.is_none_or(|value| {
        value
            .split(',')
            .any(|key_method| Some(key_method) == method)
    })
}
