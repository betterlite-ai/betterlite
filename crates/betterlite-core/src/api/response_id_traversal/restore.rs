use betterlite_models::decisions::Json;

use crate::api::response_id_codecs::encrypted::{
    decode_encrypted_item_id, unwrap_encrypted_content,
};

use super::record::Record;

pub fn restore_encrypted_item_ids(input: &mut Record) {
    let Record::List(items) = input else {
        return;
    };
    for item in items {
        let Record::Dict(dict) = item else {
            continue;
        };
        let decoded = dict
            .values
            .get("id")
            .and_then(Record::as_text)
            .filter(|text| !text.is_empty())
            .and_then(decode_encrypted_item_id);
        if let Some(item_id) = decoded {
            dict.values
                .insert("id".to_owned(), Record::Scalar(Json::Text(item_id.item_id)));
        }
        let mut unwrapped_content = None;
        if let Some(content) = dict
            .values
            .get("encrypted_content")
            .and_then(Record::as_text)
            .filter(|text| !text.is_empty())
        {
            let (_, unwrapped) = unwrap_encrypted_content(content);
            if unwrapped != content {
                unwrapped_content = Some(unwrapped);
            }
        }
        if let Some(unwrapped) = unwrapped_content {
            dict.values.insert(
                "encrypted_content".to_owned(),
                Record::Scalar(Json::Text(unwrapped)),
            );
        }
    }
}
