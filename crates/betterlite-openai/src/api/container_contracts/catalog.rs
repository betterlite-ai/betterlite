use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerEndpoint {
    pub name: String,
    pub async_name: String,
    pub path: String,
    pub method: String,
    pub path_params: Vec<String>,
    pub query_params: Vec<String>,
    pub response_type: String,
    #[serde(default)]
    pub is_multipart: bool,
    #[serde(default)]
    pub returns_binary: bool,
}

#[derive(Debug, Deserialize)]
struct ContainerEndpointsDocument {
    endpoints: Vec<ContainerEndpoint>,
}

const CONTAINER_ENDPOINTS_JSON: &str = include_str!("endpoints.json");

#[must_use]
pub fn container_endpoints() -> Vec<ContainerEndpoint> {
    match serde_json::from_str::<ContainerEndpointsDocument>(CONTAINER_ENDPOINTS_JSON) {
        Ok(document) => document.endpoints,
        Err(_) => Vec::new(),
    }
}
