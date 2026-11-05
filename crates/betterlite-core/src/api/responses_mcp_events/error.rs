#[derive(Debug, thiserror::Error)]
pub enum McpEventError {
    #[error("{message}")]
    ToolAttribute { message: String },
    #[error("sequence number {start} plus {offset} overflows")]
    SequenceOverflow { start: i64, offset: i64 },
    #[error("event field {field} must be {expected}")]
    FieldType {
        field: String,
        expected: &'static str,
    },
    #[error("event field item is not an MCP list or call item")]
    ItemShape {
        #[source]
        source: serde_json::Error,
    },
}
