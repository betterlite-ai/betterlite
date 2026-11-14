use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use serde_json::value::to_raw_value;
use serde_json::{Map, Value};

use crate::code_execution_records::CallDialect;
use crate::code_execution_records::followup_filtering;

use super::records::followup_messages;

// The follow-up keeps every other tool and swaps each code_interpreter tool for the function tool
pub fn tools(
    requested: Option<&Value>,
    dialect: CallDialect,
) -> Result<Option<Vec<FollowupMessage>>, serde_json::Error> {
    let Some(replaced) = requested.and_then(|tools| followup_filtering::tools(tools, dialect))
    else {
        return Ok(None);
    };
    followup_messages(&replaced).map(Some)
}

pub fn optional_params(optional_params: &Map<String, Value>) -> Result<JsonMap, serde_json::Error> {
    let filtered = followup_filtering::optional_params(optional_params);
    raw_map(&filtered)
}

pub fn kwargs(kwargs: &Map<String, Value>) -> Result<JsonMap, serde_json::Error> {
    let filtered = followup_filtering::agentic_loop_kwargs(kwargs);
    raw_map(&filtered)
}

fn raw_map(values: &Map<String, Value>) -> Result<JsonMap, serde_json::Error> {
    let mut mapped = JsonMap::new();
    for (key, value) in values {
        mapped.insert(key.clone(), to_raw_value(value)?);
    }
    Ok(mapped)
}

include!("followup/tests/cases.rs");
