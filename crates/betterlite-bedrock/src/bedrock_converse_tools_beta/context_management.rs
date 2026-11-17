use betterlite_anthropic::request::beta_headers::COMPACT_BETA;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::bedrock_converse_tools_beta::beta::has_beta;
use crate::bedrock_converse_tools_beta::json::{as_list, as_object, as_text};

const CONTEXT_MANAGEMENT: &str = "context_management";
const EDITS: &str = "edits";
const TYPE: &str = "type";
const COMPACT_EDIT_TYPE: &str = "compact_20260112";

// Converse keeps only compact edits; every other context_management shape is dropped without a warning.
pub fn filter_context_management(params: &mut IndexMap<String, Json>, betas: &mut Vec<Json>) {
    let Some(mut kept) = params.get(CONTEXT_MANAGEMENT).and_then(as_object).cloned() else {
        params.shift_remove(CONTEXT_MANAGEMENT);
        return;
    };
    let Some(edits) = kept.get(EDITS).and_then(as_list) else {
        params.shift_remove(CONTEXT_MANAGEMENT);
        return;
    };
    let mut retained = Vec::with_capacity(edits.len());
    for edit in edits {
        if is_compact(edit) {
            retained.push(edit.clone());
        }
    }
    if retained.is_empty() {
        params.shift_remove(CONTEXT_MANAGEMENT);
        return;
    }
    kept.insert(EDITS.to_owned(), Json::List(retained));
    params.insert(CONTEXT_MANAGEMENT.to_owned(), Json::Object(kept));
    if !has_beta(betas, COMPACT_BETA) {
        betas.push(Json::Text(COMPACT_BETA.to_owned()));
    }
}

fn is_compact(edit: &Json) -> bool {
    as_object(edit)
        .and_then(|fields| fields.get(TYPE))
        .and_then(as_text)
        == Some(COMPACT_EDIT_TYPE)
}
