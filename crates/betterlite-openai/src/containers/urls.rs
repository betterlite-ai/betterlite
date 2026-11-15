use betterlite_http::error::HttpError;
use betterlite_http::url_path::encode_segment;
use url::Url;

#[derive(Debug)]
pub enum ContainerPathSegment<'value> {
    Id {
        field_name: &'value str,
        value: &'value str,
    },
    Literal(&'value str),
}

// A resource URL keeps the base's query and fragment, since Azure bases carry ?api-version=...
pub fn resource_url(
    api_base: &str,
    segments: &[ContainerPathSegment<'_>],
) -> Result<String, HttpError> {
    let mut path = String::new();
    for segment in segments {
        path.push('/');
        match segment {
            ContainerPathSegment::Id { field_name, value } => {
                path.push_str(&encode_segment(value, field_name)?);
            }
            ContainerPathSegment::Literal(literal) => path.push_str(literal),
        }
    }
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
