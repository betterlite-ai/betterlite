use serde::Serialize;

pub trait NamedEndpoint {
    fn endpoint_name(&self) -> Option<&str>;
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RouteInfo {
    pub path: Option<String>,
    pub methods: Option<Vec<String>>,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mounted_app: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct Route {
    pub path: Option<String>,
    pub methods: Option<Vec<String>>,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub app: Option<Vec<Route>>,
}

pub fn get_app_routes<E: NamedEndpoint>(route: &Route, endpoint_route: &E) -> Vec<RouteInfo> {
    vec![RouteInfo {
        path: route.path.clone(),
        methods: route.methods.clone(),
        name: route.name.clone(),
        endpoint: route
            .endpoint
            .as_ref()
            .and_then(|_| endpoint_route.endpoint_name().map(str::to_owned)),
        mounted_app: None,
    }]
}

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
    let endpoint = sub_route
        .endpoint
        .clone()
        .or_else(|| sub_route.app.as_ref().map(|_| "Route".to_owned()))?;
    let mut path = mount_path.to_owned();
    path.push_str(sub_route.path.as_deref().unwrap_or_default());
    Some(RouteInfo {
        path: Some(path),
        methods: Some(
            sub_route
                .methods
                .clone()
                .unwrap_or_else(|| vec!["GET".to_owned(), "POST".to_owned()]),
        ),
        name: sub_route.name.clone(),
        endpoint: Some(endpoint),
        mounted_app: Some(true),
    })
}
