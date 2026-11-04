use serde::{Deserialize, Serialize};

pub trait NamedEndpoint {
    fn endpoint_name(&self) -> Result<Option<&str>, RouteInspectionError>;

    fn class_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        Ok(None)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RouteInspectionError {
    #[error("endpoint name lookup failed")]
    EndpointName,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RouteInfo {
    pub path: Option<String>,
    pub methods: Option<Vec<String>>,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mounted_app: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Route {
    pub path: Option<String>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub methods: Option<Option<Vec<String>>>,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub app_name: Option<String>,
    pub app: Option<Vec<Self>>,
}

#[must_use]
pub fn get_app_routes<E>(route: &Route, endpoint_route: &E) -> Vec<RouteInfo>
where
    E: NamedEndpoint,
{
    let route_info = RouteInfo {
        path: route.path.clone(),
        methods: route.methods.clone().flatten(),
        name: route.name.clone(),
        endpoint: route
            .endpoint
            .as_ref()
            .and_then(|context| safe_get_endpoint_name(context, endpoint_route)),
        mounted_app: None,
    };
    std::iter::once(route_info).collect()
}

#[must_use]
pub fn get_routes_for_mounted_app(route: &Route) -> Vec<RouteInfo> {
    let Some(sub_routes) = route.app.as_ref() else {
        return Vec::new();
    };
    let mount_path = route
        .path
        .as_deref()
        .unwrap_or_default()
        .trim_end_matches('/');
    sub_routes
        .iter()
        .filter_map(|sub_route| mounted_sub_route_info(mount_path, sub_route))
        .collect()
}

fn mounted_sub_route_info(mount_path: &str, sub_route: &Route) -> Option<RouteInfo> {
    let endpoint_name = sub_route.endpoint.as_deref();
    let class_name = sub_route.app.as_ref().and(sub_route.app_name.as_deref());
    if endpoint_name.is_none() && class_name.is_none() {
        return None;
    }
    let mut path = mount_path.to_owned();
    path.push_str(sub_route.path.as_deref().unwrap_or_default());
    let route_endpoint = RouteEndpoint {
        endpoint_name,
        class_name,
    };
    let safe_endpoint_name = safe_get_endpoint_name("mounted sub-route endpoint", &route_endpoint);
    Some(RouteInfo {
        path: Some(path),
        methods: sub_route
            .methods
            .clone()
            .unwrap_or_else(|| Some(vec!["GET".to_owned(), "POST".to_owned()])),
        name: sub_route.name.clone(),
        endpoint: safe_endpoint_name,
        mounted_app: Some(true),
    })
}

pub fn safe_get_endpoint_name<E>(context: &str, endpoint: &E) -> Option<String>
where
    E: NamedEndpoint,
{
    match endpoint.endpoint_name() {
        Ok(Some(name)) => Some(name.to_owned()),
        Ok(None) => match endpoint.class_name() {
            Ok(name) => name.map(str::to_owned),
            Err(error) => {
                tracing::error!(context, %error, "Error getting endpoint name for route");
                None
            }
        },
        Err(error) => {
            tracing::error!(context, %error, "Error getting endpoint name for route");
            None
        }
    }
}

struct RouteEndpoint<'endpoint> {
    endpoint_name: Option<&'endpoint str>,
    class_name: Option<&'endpoint str>,
}

impl NamedEndpoint for RouteEndpoint<'_> {
    fn endpoint_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        Ok(self.endpoint_name)
    }

    fn class_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        Ok(self.class_name)
    }
}

#[path = "route_inspection/tests.rs"]
mod tests;
