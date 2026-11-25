use betterlite_core::api::vertex_location::{
    VertexLocationError as CoreLocationError, validate_vertex_location,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum VertexLocationError {
    #[error("vertex_location is required")]
    Required,
    #[error("Invalid vertex_location format")]
    InvalidFormat,
}

// LiteLLM's get_vertex_base_url: the proxy copy keeps its own errors, so it never calls the betterlite-vertex host builder
pub fn passthrough_vertex_base_url(location: Option<&str>) -> Result<String, VertexLocationError> {
    let checked =
        validate_vertex_location(location).map_err(|error| core_location_error(&error))?;
    if checked == "global" {
        return Ok("https://aiplatform.googleapis.com/".to_owned());
    }
    if checked.contains('-') {
        return Ok(format!("https://{checked}-aiplatform.googleapis.com/"));
    }
    Ok(format!("https://aiplatform.{checked}.rep.googleapis.com/"))
}

const fn core_location_error(error: &CoreLocationError) -> VertexLocationError {
    match error {
        CoreLocationError::Required => VertexLocationError::Required,
        CoreLocationError::Invalid | CoreLocationError::Project { .. } => {
            VertexLocationError::InvalidFormat
        }
    }
}
