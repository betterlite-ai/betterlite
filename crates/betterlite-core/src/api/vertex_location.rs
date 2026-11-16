mod tests;

use thiserror::Error;

pub const DEFAULT_SPEECH_LOCATION: &str = "us";

const URL_UNSAFE_PROJECT_CHARS: [char; 9] = ['/', '?', '#', '\\', ':', ' ', '\t', '\n', '\r'];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VertexLocationError {
    #[error("vertex_location is required")]
    Required,
    #[error("invalid vertex_location format")]
    Invalid,
    #[error("the vertex_project '{project}' cannot be used in a URL")]
    Project { project: String },
}

pub fn validate_vertex_location(location: Option<&str>) -> Result<&str, VertexLocationError> {
    let Some(requested) = location else {
        return Err(VertexLocationError::Required);
    };
    // LiteLLM's ^[a-z][a-z0-9-]*$ keeps a client-supplied location from choosing the host; global passes it too
    let mut characters = requested.chars();
    let starts_with_letter = characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let rest_is_valid = characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    if starts_with_letter && rest_is_valid {
        return Ok(requested);
    }
    Err(VertexLocationError::Invalid)
}

pub fn validate_vertex_project(project: &str) -> Result<&str, VertexLocationError> {
    // an empty id, a parent path or any of these characters would move the request to another resource
    let unsafe_project = project.is_empty()
        || project.contains("..")
        || project
            .chars()
            .any(|character| URL_UNSAFE_PROJECT_CHARS.contains(&character));
    if unsafe_project {
        return Err(VertexLocationError::Project {
            project: project.to_owned(),
        });
    }
    Ok(project)
}

#[must_use]
pub fn speech_to_text_host(location: &str) -> String {
    if location == "global" {
        "speech.googleapis.com".to_owned()
    } else {
        format!("{location}-speech.googleapis.com")
    }
}
