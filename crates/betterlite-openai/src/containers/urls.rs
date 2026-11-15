use betterlite_http::url_path::encode_segment;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
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
#[must_use]
pub fn resource_url(api_base: &str, segments: &[ContainerPathSegment<'_>]) -> String {
    let mut path = String::new();
    for segment in segments {
        path.push('/');
        match segment {
            ContainerPathSegment::Id { field_name, value } => {
                path.push_str(&encode_id(value, field_name));
            }
            ContainerPathSegment::Literal(literal) => path.push_str(literal),
        }
    }
    join_container_api_base_path(api_base, &path)
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

// B3360's request transforms answer without an error channel, so the two ids B171 refuses stay fully escaped
fn encode_id(value: &str, field_name: &str) -> String {
    match encode_segment(value, field_name) {
        Ok(encoded) => encoded,
        Err(_error) => utf8_percent_encode(value, NON_ALPHANUMERIC).to_string(),
    }
}
