use betterlite_http::error::HttpError;
use betterlite_http::url_path::encode_segment;
use url::Url;

#[derive(Debug, Clone, Copy)]
pub enum ContainerResource<'id> {
    Container(&'id str),
    Files(&'id str),
    FileContent {
        container_id: &'id str,
        file_id: &'id str,
    },
}

// A resource URL keeps the base's query and fragment, since Azure bases carry ?api-version=...
pub fn resource_url(api_base: &str, resource: ContainerResource<'_>) -> Result<String, HttpError> {
    let path = match resource {
        ContainerResource::Container(container_id) => {
            format!("/{}", encode_segment(container_id, "container_id")?)
        }
        ContainerResource::Files(container_id) => {
            format!("/{}/files", encode_segment(container_id, "container_id")?)
        }
        ContainerResource::FileContent {
            container_id,
            file_id,
        } => format!(
            "/{}/files/{}/content",
            encode_segment(container_id, "container_id")?,
            encode_segment(file_id, "file_id")?
        ),
    };
    Ok(join_container_api_base_path(api_base, &path))
}

#[must_use]
pub fn join_container_api_base_path(api_base: &str, path_suffix: &str) -> String {
    let suffix = if path_suffix.starts_with('/') {
        path_suffix.to_owned()
    } else {
        format!("/{path_suffix}")
    };
    let Ok(mut url) = Url::parse(api_base) else {
        return format!("{}{suffix}", api_base.trim_end_matches('/'));
    };
    let path = format!("{}{suffix}", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.to_string()
}
