use super::support::as_object_mut;
use crate::spend_sanitize::REDACTED_BY_LITELM;
use betterlite_models::decisions::Json;
pub fn redact_vector_store(requests: &mut [Json], store_bodies: bool) {
    if store_bodies {
        return;
    }
    for request in requests {
        redact_request(request);
    }
}
fn redact_request(request: &mut Json) {
    let Some(response) = as_object_mut(request)
        .and_then(|fields| fields.get_mut("vector_store_search_response"))
        .and_then(as_object_mut)
    else {
        return;
    };
    let Some(Json::List(data)) = response.get_mut("data") else {
        return;
    };
    for response_item in data {
        redact_response_item(response_item);
    }
}
fn redact_response_item(response_item: &mut Json) {
    let Some(stored_item) = as_object_mut(response_item) else {
        return;
    };
    let Some(Json::List(content)) = stored_item.get_mut("content") else {
        return;
    };
    for content_item in content {
        if let Json::Object(stored_content) = content_item
            && stored_content.contains_key("text")
        {
            stored_content.insert("text".to_owned(), Json::Text(REDACTED_BY_LITELM.to_owned()));
        }
    }
}
