use crate::sources::parse_execution::{ParseExecutionError, parse_execution};

pub fn selection_id(value: &str) -> Result<String, ParseExecutionError> {
    let (source, team, trace_id, trace_ref) = parse_execution(value)?;
    let identity = if trace_ref.is_empty() {
        trace_id
    } else {
        trace_ref
    };
    Ok([source, team, identity].join("\0"))
}

mod tests;
