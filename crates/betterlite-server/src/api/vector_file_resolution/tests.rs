#![cfg(test)]

mod support;

mod hints;
mod overlay;
mod permissions;
mod routing;

use betterlite_models::model_registration::JsonObject;

use super::response_ids::{FileResponse, restore_file_identifiers};

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

#[test]
fn restore_file_identifiers_dict_replaces_both() {
    let mut fields = object(r#"{"id":"provider","file_id":"provider","other":"x"}"#);
    restore_file_identifiers(Some(FileResponse::Fields(&mut fields)), "managed");
    assert_eq!(
        fields,
        object(r#"{"id":"managed","file_id":"managed","other":"x"}"#)
    );
}

#[test]
fn restore_file_identifiers_dict_without_ids_unchanged() {
    let mut fields = object(r#"{"object":"vector_store.file","other":"x"}"#);
    restore_file_identifiers(Some(FileResponse::Fields(&mut fields)), "managed");
    assert_eq!(
        fields,
        object(r#"{"object":"vector_store.file","other":"x"}"#)
    );
}

#[test]
fn restore_file_identifiers_nested_ids_untouched() {
    let mut fields = object(
        r#"{"id":"provider","file_id":"provider","vector_store_id":"vs","data":[{"id":"inner"}]}"#,
    );
    restore_file_identifiers(Some(FileResponse::Fields(&mut fields)), "managed");
    assert_eq!(
        fields,
        object(
            r#"{"id":"managed","file_id":"managed","vector_store_id":"vs","data":[{"id":"inner"}]}"#
        )
    );
}

#[test]
fn restore_file_identifiers_object_id_first() {
    let mut id = String::from("provider");
    let mut file_id = String::from("provider");
    restore_file_identifiers(
        Some(FileResponse::Both {
            id: &mut id,
            file_id: &mut file_id,
        }),
        "managed",
    );
    assert_eq!(id, "managed");
    assert_eq!(file_id, "provider");
}

#[test]
fn restore_file_identifiers_object_single_attribute() {
    let mut id = String::from("provider");
    restore_file_identifiers(Some(FileResponse::Identifier(&mut id)), "managed");
    assert_eq!(id, "managed");
    let mut file_id = String::from("provider");
    restore_file_identifiers(Some(FileResponse::FileIdentifier(&mut file_id)), "managed");
    assert_eq!(file_id, "managed");
    restore_file_identifiers(None, "managed");
}
