use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolPermissionError {
    #[error("a tool permission rule is missing the required field '{field}'")]
    MissingField { field: &'static str },
    #[error("the tool permission rule field '{field}' must be {expected}")]
    WrongType {
        field: String,
        expected: &'static str,
    },
    #[error("a tool permission rule has an unknown field '{field}'")]
    UnknownField { field: String },
    #[error("the tool permission rule decision '{value}' must be \"allow\" or \"deny\"")]
    InvalidDecision { value: String },
    #[error("Each rule must specify at least a tool_name or tool_type regex")]
    MissingTarget,
    #[error("invalid regex for {field} in rule '{rule_id}': '{pattern}'")]
    InvalidRegex {
        rule_id: String,
        field: String,
        pattern: String,
        #[source]
        source: regex::Error,
    },
    #[error("the tool permission default_action '{value}' must be \"allow\" or \"deny\"")]
    InvalidDefaultAction { value: String },
    #[error("the tool permission on_disallowed_action '{value}' must be \"block\" or \"rewrite\"")]
    InvalidDisallowedAction { value: String },
}
