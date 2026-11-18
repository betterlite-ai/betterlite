use std::io;
use std::path::PathBuf;

use super::custom_routes::CustomRoutesError;
use super::definition::DefinitionError;

#[derive(Debug, thiserror::Error)]
pub enum RouteLoadError {
    #[error("SemanticGuard: unknown route template '{name}'. Available templates: {}", available.join(", "))]
    UnknownTemplate {
        name: String,
        available: Vec<&'static str>,
    },
    #[error("the built-in route template '{name}' is not valid yaml: {source}")]
    Template {
        name: &'static str,
        #[source]
        source: serde_saphyr::Error,
    },
    #[error("the built-in route template '{name}' is invalid: {source}")]
    TemplateDefinition {
        name: &'static str,
        #[source]
        source: DefinitionError,
    },
    #[error("SemanticGuard: custom routes file not found: {}", path.display())]
    FileNotFound { path: PathBuf },
    #[error("the custom routes file at {} could not be read: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the custom routes file at {} is invalid: {source}", path.display())]
    CustomRoutes {
        path: PathBuf,
        #[source]
        source: CustomRoutesError,
    },
    #[error("route definition {index} in the inline config is invalid: {source}")]
    InlineDefinition {
        index: usize,
        #[source]
        source: DefinitionError,
    },
    #[error("route {name} has a similarity threshold that is not finite")]
    NonFiniteThreshold { name: String },
}
