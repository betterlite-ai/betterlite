use crate::vertex_schema::error::VertexSchemaError;

#[derive(Debug, thiserror::Error)]
pub enum GeminiFormatError {
    #[error(transparent)]
    Schema(#[from] VertexSchemaError),
}
