use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::credentials::Credentials;

pub const DEFAULT_RUN_NAME: &str = "LLMRun";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetadataFields {
    pub project_name: String,
    pub run_name: String,
    pub run_id: Option<String>,
    pub parent_run_id: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub dotted_order: Option<String>,
}

// LangsmithLogger._extract_metadata_fields, which reads litellm_params.metadata and falls back to the credentials' project.
#[must_use]
pub fn extract_metadata_fields(
    metadata: &IndexMap<String, Json>,
    credentials: &Credentials,
    default_run_name: &str,
) -> MetadataFields {
    let field = |key: &str| metadata.get(key).and_then(text_value);
    MetadataFields {
        project_name: field("project_name").unwrap_or_else(|| credentials.project.clone()),
        run_name: field("run_name").unwrap_or_else(|| default_run_name.to_owned()),
        run_id: field("id").or_else(|| field("run_id")),
        parent_run_id: field("parent_run_id"),
        trace_id: field("trace_id"),
        session_id: field("session_id"),
        dotted_order: field("dotted_order"),
    }
}

fn text_value(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
