use regex::{NoExpand, Regex};
use url::Url;

use crate::model_urls::ApiVersion;

use super::error::EndpointTargetError;
use super::graft::parse_base;

const PROJECT_PATTERN: &str = "/projects/([^/]+)";
const LOCATION_PATTERN: &str = "/locations/([^/]+)";
const MODEL_PATTERN: &str = "/models/([^:]+)";
const ROUTE_PATTERN: &str = "/projects/[^/]+/locations/[^/]+/";

#[must_use]
pub fn get_vertex_project_id_from_url(url: &str) -> Option<&str> {
    let pattern = Regex::new(PROJECT_PATTERN).ok()?;
    pattern
        .captures(url)?
        .get(1)
        .map(|capture| capture.as_str())
}

#[must_use]
pub fn get_vertex_location_from_url(url: &str) -> Option<&str> {
    let pattern = Regex::new(LOCATION_PATTERN).ok()?;
    pattern
        .captures(url)?
        .get(1)
        .map(|capture| capture.as_str())
}

#[must_use]
pub fn get_vertex_model_id_from_url(url: &str) -> Option<&str> {
    let pattern = Regex::new(MODEL_PATTERN).ok()?;
    pattern
        .captures(url)?
        .get(1)
        .map(|capture| capture.as_str())
}

pub fn replace_route_project_location(
    requested_route: &str,
    vertex_project: &str,
    vertex_location: &str,
) -> Result<String, EndpointTargetError> {
    let pattern =
        Regex::new(ROUTE_PATTERN).map_err(|source| EndpointTargetError::Pattern { source })?;
    let replacement = format!("/projects/{vertex_project}/locations/{vertex_location}/");
    Ok(pattern
        .replace_all(requested_route, NoExpand(&replacement))
        .into_owned())
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
            replace_route_project_location(requested_route, project, location)?
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
