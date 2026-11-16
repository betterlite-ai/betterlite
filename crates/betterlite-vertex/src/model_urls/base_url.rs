use betterlite_core::api::vertex_location::validate_vertex_location;

use super::error::VertexUrlError;

pub fn get_vertex_base_url(location: Option<&str>) -> Result<String, VertexUrlError> {
    // the host is chosen only after the location passes the injection check
    Ok(host_for(validate_vertex_location(location)?))
}

#[must_use]
pub(crate) fn host_for(location: &str) -> String {
    match location {
        "global" => "https://aiplatform.googleapis.com".to_owned(),
        geography if geography.contains('-') => {
            format!("https://{geography}-aiplatform.googleapis.com")
        }
        geography => format!("https://aiplatform.{geography}.rep.googleapis.com"),
    }
}
