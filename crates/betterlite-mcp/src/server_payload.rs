pub mod names;
pub mod request;
pub mod rules;
mod tests;
use betterlite_db::mcp::table_access::McpIdentifierConflict;
use thiserror::Error;

use self::rules::identifier_conflict_message;
// every variant answers 400 except UnknownRequiredFields, which answers 500
#[derive(Debug, Error)]
pub enum PayloadError {
    #[error(
        "Server name cannot contain '{separator}'. Use an alternative character instead Found: {name}"
    )]
    SeparatorInName { separator: String, name: String },
    #[error(
        "Invalid display name '{display_name}' for tool '{tool_name}'. Display names may only contain letters, digits, underscores, and hyphens (no spaces or other special characters), since they replace the tool name sent to the LLM provider."
    )]
    DisplayName {
        display_name: String,
        tool_name: String,
    },
    #[error("Invalid MCP tool prefix '{value}' provided via {field}{warnings}")]
    ToolPrefix {
        value: String,
        field: &'static str,
        warnings: String,
    },
    #[error(
        "Invalid upstream_token_header {rendered}: must be a valid HTTP header name (RFC 7230 token, e.g. 'esb-oauth')"
    )]
    UpstreamTokenHeader { rendered: String },
    #[error("{}", identifier_conflict_message(.conflict))]
    IdentifierConflict { conflict: McpIdentifierConflict },
    #[error(
        "mcp_required_fields contains unknown field names: {unknown}. Check general_settings.mcp_required_fields in your proxy config."
    )]
    UnknownRequiredFields { unknown: String },
    #[error(
        "Submission is missing required fields: {missing}. Configure required fields via general_settings.mcp_required_fields."
    )]
    MissingRequiredFields { missing: String },
}
#[derive(Debug, Error)]
pub enum RequestError {
    #[error("{0}")]
    Value(String),
    #[error(transparent)]
    Shape(#[from] serde_json::Error),
}
// the messages carry Python repr text; an ordinary string quotes the way repr() does
pub(crate) fn quote(value: &str) -> String {
    let mark = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let escaped = value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace(mark, &format!("\\{mark}"));
    format!("{mark}{escaped}{mark}")
}
pub(crate) fn quoted_list<T: AsRef<str>, I: IntoIterator<Item = T>>(values: I) -> String {
    let rendered: Vec<String> = values
        .into_iter()
        .map(|value| quote(value.as_ref()))
        .collect();
    format!("[{}]", rendered.join(", "))
}
