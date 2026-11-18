use super::definition::{RawDefinition, RouteDefinition};
use super::error::RouteLoadError;

// the pinned LiteLLM's route_templates folder, embedded so a template never has to exist on disk
const BUILTIN_TEMPLATES: [(&str, &str); 2] = [
    (
        "prompt_injection",
        include_str!("route_templates/prompt_injection.yaml"),
    ),
    (
        "sql_injection",
        include_str!("route_templates/sql_injection.yaml"),
    ),
];

#[must_use]
pub fn list() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = BUILTIN_TEMPLATES.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    names
}

pub fn load(name: &str) -> Result<RouteDefinition, RouteLoadError> {
    let Some((known, body)) = BUILTIN_TEMPLATES
        .iter()
        .find_map(|&(known, body)| (known == name).then_some((known, body)))
    else {
        return Err(RouteLoadError::UnknownTemplate {
            name: name.to_owned(),
            available: list(),
        });
    };
    let raw: RawDefinition =
        serde_saphyr::from_slice(body.as_bytes()).map_err(|source| RouteLoadError::Template {
            name: known,
            source,
        })?;
    raw.into_definition()
        .map_err(|source| RouteLoadError::TemplateDefinition {
            name: known,
            source,
        })
}
