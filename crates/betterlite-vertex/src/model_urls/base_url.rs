use super::error::VertexUrlError;

pub fn validate_vertex_location(location: Option<&str>) -> Result<&str, VertexUrlError> {
    let Some(validated) = location else {
        return Err(VertexUrlError::LocationRequired);
    };
    if validated == "global" {
        return Ok(validated);
    }
    // LiteLLM's ^[a-z][a-z0-9-]*$ keeps a client-supplied location from choosing the host
    let mut characters = validated.chars();
    let starts_with_letter = characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let rest_is_valid = characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    if starts_with_letter && rest_is_valid {
        return Ok(validated);
    }
    Err(VertexUrlError::InvalidLocation)
}

pub fn get_vertex_base_url(location: Option<&str>) -> Result<String, VertexUrlError> {
    match validate_vertex_location(location) {
        Ok(validated) => Ok(host_for(validated)),
        Err(error) => Err(error),
    }
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
