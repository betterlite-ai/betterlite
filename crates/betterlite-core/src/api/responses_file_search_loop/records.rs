use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResult;
use rust_decimal::Decimal;

use crate::api::responses_tool_namespaces::values::Fields;

use super::ports::FollowUpRequest;

// verbose_logger is logging.getLogger("LiteLLM"); the emulation logs its warnings through it
pub const FILE_SEARCH_LOGGER: &str = "LiteLLM";
pub const WARNING_LEVEL: &str = "WARNING";

// uuid.uuid4().hex behind __call_id (emulated_handler.py:571, L585)
pub type IdSource = Arc<dyn Fn() -> String + Send + Sync>;

// datetime.now(timezone.utc).timestamp() behind created_at (emulated_handler.py:364)
pub type ClockSource = Arc<dyn Fn() -> Decimal + Send + Sync>;

// who the search runs as; LiteLLM's two-argument asearch drops this, betterlite keeps it on every search
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchAuthorization {
    pub user: Option<String>,
    pub team: Option<String>,
}

// one provider call with the arguments it carried and is_internal_call as the provider saw it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUpCall {
    pub input: Json,
    pub model: String,
    pub tools: Option<Vec<Json>>,
    pub kwargs: Fields,
    pub internal: bool,
}

impl FollowUpCall {
    #[must_use]
    pub fn request(&self) -> FollowUpRequest<'_> {
        FollowUpRequest {
            input: &self.input,
            model: &self.model,
            tools: self.tools.as_deref(),
            kwargs: &self.kwargs,
            internal: self.internal,
        }
    }
}

// what _execute_file_search_tool_calls (L440-L486) returns, with the id the round reports under
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedToolCalls {
    pub call_id: String,
    pub queries: Vec<String>,
    pub results: Vec<VectorStoreSearchResult>,
    pub outputs: Vec<Json>,
    pub searches: Vec<UpstreamSearch>,
    pub diagnostics: Vec<FileSearchDiagnostic>,
}

// one asearch call as it went out, with what came back or the failure that was logged
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamSearch {
    pub vector_store_id: String,
    pub query: String,
    pub result_count: usize,
    pub failed: bool,
}

// one verbose_logger.warning the emulation emitted
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSearchDiagnostic {
    pub level: &'static str,
    pub message: String,
}

// the warning texts at emulated_handler.py:171-L176 and L463-L468, rendered as Python formats them
#[derive(Debug, Clone)]
pub enum DiagnosticMessage<'message> {
    SearchFailed {
        query: &'message str,
        vector_store_id: &'message str,
        error: &'message str,
    },
    StoreOutside {
        value: &'message Json,
        known: &'message [String],
    },
}

impl DiagnosticMessage<'_> {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::SearchFailed {
                query,
                vector_store_id,
                error,
            } => format!(
                "file_search emulated: search failed for query='{query}', \
                 vector_store_id='{vector_store_id}': {error}"
            ),
            Self::StoreOutside { value, known } => {
                let shown_value = display_value(value);
                // str() of the request's ids, which is repr() of a list of strings
                let listed = Json::List(known.iter().map(|id| Json::Text(id.clone())).collect());
                let shown_ids = display_value(&listed);
                format!(
                    "file_search emulated: model picked vector_store_id={shown_value} outside the \
                     request's vector_store_ids {shown_ids}; searching the request's stores instead"
                )
            }
        }
    }
}

// repr() of the JSON value that %r renders in the warning text
fn display_value(value: &Json) -> String {
    match value {
        Json::Text(text) => quote(text),
        Json::Bool(flag) => String::from(if *flag { "True" } else { "False" }),
        Json::Number(number) => number.to_string(),
        Json::Null => String::from("None"),
        Json::List(items) => {
            let shown: Vec<String> = items.iter().map(display_value).collect();
            format!("[{}]", shown.join(", "))
        }
        Json::Object(fields) => {
            let shown: Vec<String> = fields
                .iter()
                .map(|(key, field)| format!("{}: {}", quote(key), display_value(field)))
                .collect();
            format!("{{{}}}", shown.join(", "))
        }
    }
}

// Python's repr of a string: single quotes unless that forces escaping and double quotes would not
fn quote(text: &str) -> String {
    if text.contains('\'') && !text.contains('"') {
        return format!("\"{text}\"");
    }
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("'{escaped}'")
}
