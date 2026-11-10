use betterlite_models::decisions::Json;

pub const VERTEX_METADATA_FIELDS: [&str; 5] = [
    "vertex_ai_grounding_metadata",
    "vertex_ai_url_context_metadata",
    "vertex_ai_safety_ratings",
    "vertex_ai_safety_results",
    "vertex_ai_citation_metadata",
];

pub fn redact_logged_object_metadata(object: &mut Json) {
    let Json::Object(fields) = object else {
        return;
    };
    for field in VERTEX_METADATA_FIELDS {
        if let Some(value) = fields.get_mut(field) {
            *value = Json::List(Vec::new());
        }
    }
    if let Some(Json::Object(hidden)) = fields.get_mut("_hidden_params") {
        for field in VERTEX_METADATA_FIELDS {
            hidden.shift_remove(field);
        }
    }
}

pub fn redact_litellm_params_metadata(model_call_details: &mut Json) {
    let Json::Object(details) = model_call_details else {
        return;
    };
    let Some(Json::Object(litellm_params)) = details.get_mut("litellm_params") else {
        return;
    };
    // success_handler copies the response's hidden params onto both metadata dicts before redaction runs
    for metadata_key in ["metadata", "litellm_metadata"] {
        let Some(Json::Object(metadata)) = litellm_params.get_mut(metadata_key) else {
            continue;
        };
        let Some(Json::Object(hidden)) = metadata.get_mut("hidden_params") else {
            continue;
        };
        for field in VERTEX_METADATA_FIELDS {
            hidden.shift_remove(field);
        }
    }
}
