use super::ApiVersion;
use super::base_url::host_for;
use super::model_name::is_decimal;

#[derive(Clone, Copy)]
pub(crate) enum VertexCollection {
    Endpoints,
    Publishers,
}

impl VertexCollection {
    const fn segment(self) -> &'static str {
        match self {
            Self::Endpoints => "endpoints",
            Self::Publishers => "publishers/google/models",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct VertexPath<'text> {
    pub(crate) model: &'text str,
    pub(crate) project: &'text str,
    pub(crate) location: &'text str,
}

pub(crate) fn model_path(
    path: &VertexPath<'_>,
    api_version: ApiVersion,
    publisher_version: &str,
    endpoint: &str,
) -> String {
    if is_decimal(path.model) {
        resource_path(
            path,
            api_version.as_str(),
            endpoint,
            VertexCollection::Endpoints,
        )
    } else {
        resource_path(
            path,
            publisher_version,
            endpoint,
            VertexCollection::Publishers,
        )
    }
}

pub(crate) fn resource_path(
    path: &VertexPath<'_>,
    version: &str,
    endpoint: &str,
    collection: VertexCollection,
) -> String {
    let VertexPath {
        model,
        project,
        location,
    } = *path;
    let base_url = host_for(location);
    let segment = collection.segment();
    format!(
        "{base_url}/{version}/projects/{project}/locations/{location}/{segment}/{model}:{endpoint}"
    )
}
