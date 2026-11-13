use betterlite_prompts::arguments::json::{Json, Object};

const BILLING_PREFIX: &str = "x-anthropic-billing-header:";
const TEXT_TYPE: &str = "text";

// LiteLLM keeps a system value it has nothing to filter from, and reports a null one as absent
#[must_use]
pub fn filter_billing_headers(system: &Json) -> Option<Json> {
    match system {
        Json::Null => None,
        Json::Text(text) => {
            if text.starts_with(BILLING_PREFIX) {
                None
            } else {
                Some(system.clone())
            }
        }
        Json::List(items) => {
            let kept: Vec<Json> = items
                .iter()
                .filter(|block| !is_billing_block(block))
                .cloned()
                .collect();
            (!kept.is_empty()).then_some(Json::List(kept))
        }
        Json::Bool(_) | Json::Number(_) | Json::Object(_) => Some(system.clone()),
    }
}

fn is_billing_block(block: &Json) -> bool {
    let Some(fields) = block.as_object() else {
        return false;
    };
    if fields.get("type").and_then(Json::as_str) != Some(TEXT_TYPE) {
        return false;
    }
    fields
        .get("text")
        .and_then(Json::as_str)
        .is_some_and(|text| text.starts_with(BILLING_PREFIX))
}

// Vertex AI and Foundry reject the cross-request scope field that the first-party API accepts
pub fn remove_cache_scope(request: &mut Object) {
    if let Some(Json::List(system)) = request.get_mut("system") {
        strip_scope(system);
    }
    let Some(Json::List(messages)) = request.get_mut("messages") else {
        return;
    };
    for message in messages {
        let Json::Object(fields) = message else {
            continue;
        };
        if let Some(Json::List(content)) = fields.get_mut("content") {
            strip_scope(content);
        }
    }
}

fn strip_scope(blocks: &mut [Json]) {
    for block in blocks {
        let Json::Object(fields) = block else {
            continue;
        };
        let Some(Json::Object(cache_control)) = fields.get_mut("cache_control") else {
            continue;
        };
        cache_control.shift_remove("scope");
    }
}
