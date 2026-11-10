use crate::definitions::error::DefinitionsError;

#[derive(Debug, thiserror::Error)]
pub enum BedrockToolError {
    #[error("tool {index} is not a JSON object")]
    ToolNotObject { index: usize },
    #[error("tool field {field} has a type a Bedrock tool spec cannot hold")]
    WrongType { field: &'static str },
    #[error("tool parameter schema could not be expanded: {source}")]
    Schema {
        #[source]
        source: DefinitionsError,
    },
    #[error("tool parameter schemas exceed the {limit}-byte expansion budget")]
    SchemaBudget { limit: usize },
    #[error("tool list is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}
