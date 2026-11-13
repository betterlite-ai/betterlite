use thiserror::Error;

#[derive(Debug, Error)]
pub enum SamplingConvertError {
    #[error("tool call arguments: Input should be a valid dictionary")]
    ToolArgumentsNotObject,
    #[error("a JSON object was expected: Input should be a valid dictionary")]
    JsonObjectExpected,
    #[error("tool call arguments could not be written as JSON")]
    ToolArgumentsEncoding {
        #[source]
        source: serde_json::Error,
    },
    #[error("MCP sampling priority scores overflowed the decimal range")]
    PriorityScoreOverflow,
    #[error(
        "no model could be resolved for MCP sampling; set default_mcp_sampling_model in the configuration"
    )]
    NoModelResolved,
}
