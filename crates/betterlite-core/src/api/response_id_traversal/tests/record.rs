use crate::api::response_id_traversal::record::Record;

use super::fixtures::{dict, list, null, number, object, text};

#[test]
fn mappings_and_scalars_serialize_unchanged() {
    let record = dict(vec![
        ("container_id", text("c1")),
        ("count", number(5)),
        ("empty", null()),
        ("items", list(vec![text("x"), number(2)])),
    ]);
    assert_eq!(
        serde_json::to_string(&record).unwrap(),
        r#"{"container_id":"c1","count":5,"empty":null,"items":["x",2]}"#
    );
}

#[test]
fn typed_objects_serialize_with_their_marker() {
    let record = object(vec![("container_id", text("c1"))], &["container_id", "id"]);
    assert_eq!(
        serde_json::to_string(&record).unwrap(),
        r#"{"__object__":{"fields":{"container_id":"c1"},"readonly":["container_id","id"]}}"#
    );
}

#[test]
fn wiring_a_typed_object_back_keeps_its_read_only_fields() {
    let record = object(vec![("id", text("r"))], &["id"]);
    let encoded = serde_json::to_string(&record).unwrap();
    assert_eq!(serde_json::from_str::<Record>(&encoded).unwrap(), record);
}

#[test]
fn a_plain_mapping_is_not_mistaken_for_a_typed_object() {
    let wire = r#"{"__object__":"not an object payload","other":1}"#;
    assert_eq!(
        serde_json::from_str::<Record>(wire).unwrap(),
        dict(vec![
            ("__object__", text("not an object payload")),
            ("other", number(1))
        ])
    );
}
