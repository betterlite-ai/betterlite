#[derive(Debug, thiserror::Error)]
pub enum McpEventError {
    #[error("{message}")]
    ToolAttribute { message: String },
    #[error("sequence number {start} plus {offset} overflows")]
    SequenceOverflow { start: i64, offset: i64 },
}
