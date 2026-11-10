use super::support::as_mapping_mut;
use crate::spend_sanitize::REDACTED_BY_LITELM;
use crate::spend_sanitize::value::SpendValue;

pub fn redact_vector_store(requests: &mut [SpendValue], store_bodies: bool) {
    if store_bodies {
        return;
    }
    for request in requests {
        redact_request(request);
    }
}

fn redact_request(request: &mut SpendValue) {
    let Some(stored_request) = as_mapping_mut(request) else {
        return;
    };
    let Some(response) = stored_request
        .entries
        .get_mut("vector_store_search_response")
        .and_then(as_mapping_mut)
    else {
        return;
    };
    let Some(SpendValue::List(data)) = response.entries.get_mut("data") else {
        return;
    };
    for response_item in &mut data.items {
        redact_response_item(response_item);
    }
}

fn redact_response_item(response_item: &mut SpendValue) {
    let Some(stored_item) = as_mapping_mut(response_item) else {
        return;
    };
    let Some(SpendValue::List(content)) = stored_item.entries.get_mut("content") else {
        return;
    };
    for content_item in &mut content.items {
        redact_content_item(content_item);
    }
}

fn redact_content_item(content_item: &mut SpendValue) {
    let Some(stored_content) = as_mapping_mut(content_item) else {
        return;
    };
    if stored_content.entries.contains_key("text") {
        stored_content.entries.insert(
            "text".to_owned(),
            SpendValue::Text(REDACTED_BY_LITELM.to_owned()),
        );
    }
}
