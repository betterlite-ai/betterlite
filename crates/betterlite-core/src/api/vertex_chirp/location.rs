use super::error::ChirpAdapterError;

pub(super) const DEFAULT_LOCATION: &str = "us";

const URL_UNSAFE_PROJECT_CHARS: [char; 9] = ['/', '?', '#', '\\', ':', ' ', '\t', '\n', '\r'];

pub(super) fn validate_project(project: &str) -> Result<String, ChirpAdapterError> {
    // an empty id, a parent path or any of these characters would move the request to another resource
    let unsafe_project = project.is_empty()
        || project.contains("..")
        || project
            .chars()
            .any(|character| URL_UNSAFE_PROJECT_CHARS.contains(&character));
    if unsafe_project {
        return Err(ChirpAdapterError::Project {
            project: project.to_owned(),
        });
    }
    Ok(project.to_owned())
}

pub(super) fn validate_location(location: Option<&str>) -> Result<String, ChirpAdapterError> {
    let requested = location.unwrap_or(DEFAULT_LOCATION);
    // LiteLLM's ^[a-z][a-z0-9-]*$ rejects host injection like attacker.example/ or evil.com#
    let mut characters = requested.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        });
    if !valid {
        return Err(ChirpAdapterError::Location);
    }
    Ok(requested.to_owned())
}
