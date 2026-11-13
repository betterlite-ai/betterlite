use url::Url;

use super::error::EndpointTargetError;

pub(crate) fn parse_base(api_base: &str) -> Result<Url, EndpointTargetError> {
    let parsed = Url::parse(api_base).map_err(|source| EndpointTargetError::BaseUrl { source })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(EndpointTargetError::BaseScheme);
    }
    Ok(parsed)
}

pub fn graft_default_vertex_path(
    api_base: &str,
    default_url: &str,
) -> Result<String, EndpointTargetError> {
    let mut base = parse_base(api_base)?;
    let default =
        Url::parse(default_url).map_err(|source| EndpointTargetError::DefaultUrl { source })?;
    let mut path = base.path().trim_end_matches('/').to_owned();
    path.push('/');
    let segments: Vec<&str> = default.path().trim_start_matches('/').split('/').collect();
    let starts_with_version = matches!(segments.first(), Some(&"v1" | &"v1beta1"));
    let grafted = if starts_with_version {
        segments.get(1..).unwrap_or(&[])
    } else {
        segments.as_slice()
    };
    path.push_str(&grafted.join("/"));
    base.set_path(&path);
    Ok(base.to_string())
}
