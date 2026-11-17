use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::vector_stores::fields::{RetrievalFields, attach_to_choices, from_call_details};
use crate::api::vector_stores::{Decoration, OBJECT, RESPONSES_OBJECT, SEARCH_FAILURES};

const MESSAGE: &str = "message";
const NO_LOGGING: &str = "No litellm_logging_obj in request_data";
const NO_FIELDS: &str = "No search results or search failures found";
const ERROR_PREFIX: &str = "Error adding search results to response: ";

// Err is the None LiteLLM returns: it decorated nothing, or it failed after an earlier choice (vector_store_pre_call_hook.py:432-470).
fn metadata_outcome(
    request_data: &IndexMap<String, Json>,
) -> Result<RetrievalFields<'_>, Decoration> {
    let fields = match from_call_details(request_data) {
        Ok(Some(fields)) => fields,
        Ok(None) => {
            return Err(Decoration {
                returns_answer: false,
                diagnostics: vec![NO_LOGGING.to_owned()],
            });
        }
        Err(error) => {
            return Err(Decoration {
                returns_answer: false,
                diagnostics: vec![format!("{ERROR_PREFIX}{error}")],
            });
        }
    };
    if fields.search_results.is_empty() && fields.failures.is_empty() {
        return Err(Decoration {
            returns_answer: false,
            diagnostics: vec![NO_FIELDS.to_owned()],
        });
    }
    Ok(fields)
}

pub fn decorate_answer(request_data: &IndexMap<String, Json>, answer: &mut Json) -> Decoration {
    let fields = match metadata_outcome(request_data) {
        Ok(fields) => fields,
        Err(outcome) => return outcome,
    };
    let Json::Object(record) = answer else {
        return Decoration {
            returns_answer: true,
            diagnostics: Vec::new(),
        };
    };
    if is_responses_record(record) {
        if !fields.failures.is_empty() {
            record.insert(
                SEARCH_FAILURES.to_owned(),
                Json::List(fields.failures.to_vec()),
            );
        }
        return Decoration {
            returns_answer: true,
            diagnostics: Vec::new(),
        };
    }
    if let Err(error) = attach_to_choices(record, MESSAGE, fields) {
        return Decoration {
            returns_answer: false,
            diagnostics: vec![format!("{ERROR_PREFIX}{error}")],
        };
    }
    Decoration {
        returns_answer: true,
        diagnostics: Vec::new(),
    }
}

fn is_responses_record(record: &IndexMap<String, Json>) -> bool {
    let Some(Json::Text(marker)) = record.get(OBJECT) else {
        return false;
    };
    marker.as_str() == RESPONSES_OBJECT
}
