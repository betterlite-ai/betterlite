use crate::api::response_id_codecs::encrypted::{build_encrypted_item_id, wrap_encrypted_content};

use super::record::Record;

pub fn encode_encrypted_item_ids(response: &mut Record, model_id: Option<&str>) {
    let Some(known_model) = model_id.filter(|candidate| !candidate.is_empty()) else {
        return;
    };
    let Some(Record::List(items)) = response.field_mut("output") else {
        return;
    };
    for item in items {
        encode_item(item, known_model);
    }
}

fn encode_item(item: &mut Record, model_id: &str) {
    let original_id = item
        .field("id")
        .and_then(Record::as_text)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let cipher = item
        .field("encrypted_content")
        .and_then(Record::as_text)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let Some(plain) = cipher else {
        return;
    };
    // LiteLLM swallows the AttributeError a read-only attribute raises, then still tries the id
    let _cipher_write = item.set_text(
        "encrypted_content",
        wrap_encrypted_content(&plain, model_id),
    );
    if let Some(previous_id) = original_id {
        let _id_write = item.set_text("id", build_encrypted_item_id(model_id, &previous_id));
    }
}
