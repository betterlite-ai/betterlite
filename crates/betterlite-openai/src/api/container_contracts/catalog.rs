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

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("the embedded container endpoint catalog does not parse")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
}

const CONTAINER_ENDPOINTS_JSON: &str = include_str!("fixtures/endpoints.json");

pub fn container_endpoints() -> Result<Vec<ContainerEndpoint>, CatalogError> {
    match serde_json::from_str::<ContainerEndpointsDocument>(CONTAINER_ENDPOINTS_JSON) {
        Ok(document) => Ok(document.endpoints),
        Err(source) => Err(CatalogError::Parse { source }),
    }
}
