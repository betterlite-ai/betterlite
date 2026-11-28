use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::messages_compaction::normalize::normalize_context_edit_shapes;
use crate::api::messages_context_clear::constants::COMPACT_EDIT_TYPE;

// the one parameter a caller can drop to opt out of the polyfill
const CONTEXT_MANAGEMENT_PARAM: &str = "context_management";

// what the wrapper predicts about one request: will a compact editor run, and was any other edit asked for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditPrediction {
    pub compact: bool,
    pub non_compact: bool,
}

// _polyfill_will_run plus _spec_has_non_compact_edits, both read off the dispatcher's own normalizer
#[must_use]
pub fn predict_context_edit_execution(
    context_management_spec: Option<&Json>,
    additional_drop_params: Option<&Json>,
) -> EditPrediction {
    let Some(spec) = runnable_spec(context_management_spec, additional_drop_params) else {
        return EditPrediction {
            compact: false,
            non_compact: false,
        };
    };
    let edits: Vec<IndexMap<String, Json>> =
        normalize_context_edit_shapes(Some(spec)).unwrap_or_default();
    EditPrediction {
        compact: edits.iter().any(|edit| {
            matches!(edit.get("type"), Some(Json::Text(name)) if name.as_str() == COMPACT_EDIT_TYPE)
        }),
        non_compact: edits.iter().any(|edit| {
            matches!(edit.get("type"), Some(Json::Text(name)) if name.as_str() != COMPACT_EDIT_TYPE)
        }),
    }
}

// the spec the dispatcher would read: a dropped parameter or an empty spec skips the polyfill entirely
#[must_use]
pub(crate) fn runnable_spec<'spec>(
    context_management_spec: Option<&'spec Json>,
    additional_drop_params: Option<&Json>,
) -> Option<&'spec Json> {
    if dropped_by_param(additional_drop_params) {
        return None;
    }
    context_management_spec.filter(|spec| spec_present(spec))
}

// Python's "if not spec" over the spec's declared dict-or-list shape
fn spec_present(spec: &Json) -> bool {
    match spec {
        Json::Null => false,
        Json::Object(fields) => !fields.is_empty(),
        Json::List(entries) => !entries.is_empty(),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) => true,
    }
}

// _context_management_explicitly_dropped: only a list can name the parameter, and only text entries match
fn dropped_by_param(additional_drop_params: Option<&Json>) -> bool {
    let Some(Json::List(entries)) = additional_drop_params else {
        return false;
    };
    entries
        .iter()
        .any(|entry| matches!(entry, Json::Text(name) if name.as_str() == CONTEXT_MANAGEMENT_PARAM))
}
