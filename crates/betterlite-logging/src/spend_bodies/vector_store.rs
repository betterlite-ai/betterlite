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
    let SpendValue::Mapping(stored_request) = request else {
        return;
    };
    let Some(response) = stored_request
        .entries
        .get_mut("vector_store_search_response")
    else {
        return;
    };
    let SpendValue::Mapping(stored_response) = response else {
        return;
    };
    let Some(data) = stored_response.entries.get_mut("data") else {
        return;
    };
    let SpendValue::List(items) = data else {
        return;
    };
    for response_item in &mut items.items {
        redact_response_item(response_item);
    }
}

fn redact_response_item(response_item: &mut SpendValue) {
    let SpendValue::Mapping(stored_item) = response_item else {
        return;
    };
    let Some(content) = stored_item.entries.get_mut("content") else {
        return;
    };
    let SpendValue::List(items) = content else {
        return;
    };
    for content_item in &mut items.items {
        redact_content_item(content_item);
    }
}

fn redact_content_item(content_item: &mut SpendValue) {
    let SpendValue::Mapping(stored_content) = content_item else {
        return;
    };
    if stored_content.entries.contains_key("text") {
        stored_content.entries.insert(
            "text".to_owned(),
            SpendValue::Text(REDACTED_BY_LITELM.to_owned()),
        );
    }
}
