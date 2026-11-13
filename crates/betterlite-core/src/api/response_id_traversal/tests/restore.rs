use rstest::rstest;

use crate::api::response_id_codecs::encrypted::{build_encrypted_item_id, wrap_encrypted_content};
use crate::api::response_id_traversal::record::Record;
use crate::api::response_id_traversal::restore::restore_encrypted_item_ids;

use super::fixtures::{dict, list, number, object, text};

fn first_item(input: &Record) -> Option<&Record> {
    match input {
        Record::List(items) => items.first(),
        Record::Null
        | Record::Bool(_)
        | Record::Number(_)
        | Record::Text(_)
        | Record::Dict(_)
        | Record::Object { .. } => None,
    }
}

#[test]
fn restores_ids_and_contents() {
    let mut input = list(vec![dict(vec![
        ("id", text(&build_encrypted_item_id("dep_a", "rs_a"))),
        (
            "encrypted_content",
            text(&wrap_encrypted_content("cipher", "dep_a")),
        ),
    ])]);
    restore_encrypted_item_ids(&mut input);
    let Some(item) = first_item(&input) else {
        panic!("the item should stay");
    };
    assert_eq!(item.text_field("id"), Some("rs_a"));
    assert_eq!(item.text_field("encrypted_content"), Some("cipher"));
}

#[test]
fn visits_dictionary_items_only() {
    let encoded = build_encrypted_item_id("dep_a", "rs_a");
    let wrapped = wrap_encrypted_content("cipher", "dep_a");
    let mut input = list(vec![
        text("plain"),
        object(
            vec![
                ("id", text(&encoded)),
                ("encrypted_content", text(&wrapped)),
            ],
            &[],
        ),
        list(vec![dict(vec![("id", text(&encoded))])]),
        dict(vec![("id", number(7)), ("encrypted_content", number(8))]),
    ]);
    let original = input.clone();
    restore_encrypted_item_ids(&mut input);
    assert_eq!(input, original);
}

#[rstest]
#[case::mapping(dict(vec![("id", text("encitem_bm9wZQ=="))]))]
#[case::text(text("encitem_bm9wZQ=="))]
#[case::null(Record::Null)]
fn leaves_a_non_list_input_alone(#[case] mut input: Record) {
    let original = input.clone();
    restore_encrypted_item_ids(&mut input);
    assert_eq!(input, original);
}

#[test]
fn unwraps_one_layer_only() {
    let inner = wrap_encrypted_content("cipher", "dep_a");
    let outer = wrap_encrypted_content(&inner, "dep_b");
    let mut input = list(vec![dict(vec![("encrypted_content", text(&outer))])]);
    restore_encrypted_item_ids(&mut input);
    let Some(item) = first_item(&input) else {
        panic!("the item should stay");
    };
    assert_eq!(item.text_field("encrypted_content"), Some(inner.as_str()));
}

#[test]
fn keeps_values_that_are_not_wrappers() {
    let mut input = list(vec![dict(vec![
        ("id", text("resp_plain")),
        ("encrypted_content", text("litellm_enc:missing-separator")),
    ])]);
    let original = input.clone();
    restore_encrypted_item_ids(&mut input);
    assert_eq!(input, original);
}

#[test]
fn skips_empty_fields() {
    let mut input = list(vec![dict(vec![
        ("id", text("")),
        ("encrypted_content", text("")),
    ])]);
    let original = input.clone();
    restore_encrypted_item_ids(&mut input);
    assert_eq!(input, original);
}
