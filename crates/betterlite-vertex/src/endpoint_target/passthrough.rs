use url::Url;

use crate::model_urls::ApiVersion;

use super::error::EndpointTargetError;
use super::graft::parse_base;

#[must_use]
pub fn get_vertex_project_id_from_url(url: &str) -> Option<&str> {
    url.split("/projects/")
        .skip(1)
        .find_map(|rest| rest.split('/').next().filter(|value| !value.is_empty()))
}

#[must_use]
pub fn get_vertex_location_from_url(url: &str) -> Option<&str> {
    url.split("/locations/")
        .skip(1)
        .find_map(|rest| rest.split('/').next().filter(|value| !value.is_empty()))
}

#[must_use]
pub fn get_vertex_model_id_from_url(url: &str) -> Option<&str> {
    url.split("/models/")
        .skip(1)
        .find_map(|rest| rest.split(':').next().filter(|value| !value.is_empty()))
}

#[must_use]
pub fn replace_route_project_location(
    requested_route: &str,
    vertex_project: &str,
    vertex_location: &str,
) -> String {
    let mut replaced = String::with_capacity(requested_route.len());
    let mut remaining = requested_route;
    while let Some((before, rest)) = find_project_location_pair(remaining) {
        replaced.push_str(before);
        replaced.push_str("/projects/");
        replaced.push_str(vertex_project);
        replaced.push_str("/locations/");
        replaced.push_str(vertex_location);
        replaced.push('/');
        remaining = rest;
    }
    replaced.push_str(remaining);
    replaced
}

#[must_use]
pub fn api_version_for_route(requested_route: &str) -> ApiVersion {
    // LiteLLM searches for cachedContent case-sensitively, so cachedContents matches too
    if requested_route.contains("cachedContent") {
        ApiVersion::V1Beta1
    } else {
        ApiVersion::V1
    }
}

#[must_use]
pub fn with_api_version(requested_route: &str) -> String {
    if !requested_route.starts_with("/projects/") {
        return requested_route.to_owned();
    }
    format!(
        "/{}{requested_route}",
        api_version_for_route(requested_route).as_str()
    )
}

pub fn construct_target_url(
    base_url: &str,
    requested_route: &str,
    vertex_project: Option<&str>,
    vertex_location: Option<&str>,
) -> Result<String, EndpointTargetError> {
    if requested_route.contains('?')
        || requested_route.contains('#')
        || requested_route.chars().any(char::is_control)
    {
        return Err(EndpointTargetError::Route);
    }
    let mut base = parse_base(base_url)?;
    if requested_route.contains("locations") {
        return locations_route_target(&mut base, requested_route, vertex_project, vertex_location);
    }
    let project = vertex_project
        .filter(|value| !value.is_empty())
        .ok_or(EndpointTargetError::Project)?;
    let location = vertex_location
        .filter(|value| !value.is_empty())
        .ok_or(EndpointTargetError::Location)?;
    let (version, route) = route_with_version(requested_route);
    let path = format!(
        "/{}/projects/{project}/locations/{location}{route}",
        version.as_str()
    );
    apply_path(&mut base, &path)
}

fn locations_route_target(
    base: &mut Url,
    requested_route: &str,
    vertex_project: Option<&str>,
    vertex_location: Option<&str>,
) -> Result<String, EndpointTargetError> {
    let replacements = vertex_project
        .filter(|value| !value.is_empty())
        .zip(vertex_location.filter(|value| !value.is_empty()));
    let targeted = match replacements {
        Some((project, location)) => {
            replace_route_project_location(requested_route, project, location)
        }
        None => requested_route.to_owned(),
    };
    apply_path(base, &with_api_version(&targeted))
}

fn route_with_version(requested_route: &str) -> (ApiVersion, String) {
    if let Some(rest) = requested_route.strip_prefix("/v1/") {
        return (ApiVersion::V1, format!("/{rest}"));
    }
    if let Some(rest) = requested_route.strip_prefix("/v1beta1/") {
        return (ApiVersion::V1Beta1, format!("/{rest}"));
    }
    (
        api_version_for_route(requested_route),
        requested_route.to_owned(),
    )
}

// httpx normalizes the path it copies, so the parser removes any dot segments the route carried
fn apply_path(base: &mut Url, path: &str) -> Result<String, EndpointTargetError> {
    base.set_path(path);
    let normalized =
        Url::parse(base.as_ref()).map_err(|source| EndpointTargetError::BaseUrl { source })?;
    Ok(normalized.to_string())
}

fn find_project_location_pair(text: &str) -> Option<(&str, &str)> {
    for (index, _marker) in text.match_indices("/projects/") {
        let after_marker = text.get(index.checked_add("/projects/".len())?..)?;
        let (project, after_project) = after_marker.split_once("/locations/")?;
        if project.is_empty() || project.contains('/') {
            continue;
        }
        let (location, after_location) = after_project.split_once('/')?;
        if location.is_empty() {
            continue;
        }
        let end = text.len().saturating_sub(after_location.len());
        return Some((text.get(..index)?, text.get(end..)?));
    }
    None
}
