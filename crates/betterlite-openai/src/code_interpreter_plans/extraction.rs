use serde_json::{Map, Value};

use crate::code_execution_records::SESSION_SCOPED_KEY;
use crate::code_execution_records::helpers::extract_identity;

// LiteLLM writes this marker only as a boolean true, so a present marker is the session scope
#[must_use]
pub fn container_identity(kwargs: &Map<String, Value>) -> Option<String> {
    if !matches!(kwargs.get(SESSION_SCOPED_KEY), Some(Value::Bool(true))) {
        return None;
    }
    Some(extract_identity(kwargs))
}

include!("extraction/tests/cases.rs");
