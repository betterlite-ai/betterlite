use rstest::rstest;

use crate::api::response_id_codecs::encrypted::{build_encrypted_item_id, wrap_encrypted_content};
use crate::api::response_id_traversal::encrypted::encode_encrypted_item_ids;
use crate::api::response_id_traversal::record::Record;

use super::fixtures::{capture, dict, list, null, number, object, output, text, text_field};

fn item_id(response: &Record, index: usize) -> Option<String> {
    let Some(Record::List(items)) = response.field("output") else {
        return None;
    };
    text_field(items.get(index)?, "id")
}

fn item_content(response: &Record, index: usize) -> Option<String> {
    let Some(Record::List(items)) = response.field("output") else {
        return None;
    };
    text_field(items.get(index)?, "encrypted_content")
}

#[test]
fn wraps_content_and_ids_for_dict_items() {
    let mut response = output(vec![
        dict(vec![
            ("id", text("rs_a")),
            ("encrypted_content", text("cipher")),
        ]),
        dict(vec![("encrypted_content", text("cipher_b"))]),
    ]);
    encode_encrypted_item_ids(&mut response, Some("dep_a"));
    assert_eq!(
        item_content(&response, 0),
        Some(wrap_encrypted_content("cipher", "dep_a"))
    );
    assert_eq!(
        item_id(&response, 0),
        Some(build_encrypted_item_id("dep_a", "rs_a"))
    );
    assert_eq!(
        item_content(&response, 1),
        Some(wrap_encrypted_content("cipher_b", "dep_a"))
    );
    assert_eq!(item_id(&response, 1), None);
}

#[test]
fn wraps_content_and_ids_for_object_items() {
    let mut response = output(vec![object(
        vec![("id", text("rs_a")), ("encrypted_content", text("cipher"))],
        &[],
    )]);
    encode_encrypted_item_ids(&mut response, Some("dep_a"));
    assert_eq!(
        item_content(&response, 0),
        Some(wrap_encrypted_content("cipher", "dep_a"))
    );
    assert_eq!(
        item_id(&response, 0),
        Some(build_encrypted_item_id("dep_a", "rs_a"))
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn skips_an_empty_model(#[case] model_id: Option<&str>) {
    let mut response = output(vec![dict(vec![
        ("id", text("rs_a")),
        ("encrypted_content", text("cipher")),
    ])]);
    let original = response.clone();
    encode_encrypted_item_ids(&mut response, model_id);
    assert_eq!(response, original);
}

#[rstest]
#[case::text(text("none"))]
#[case::mapping(dict(vec![("id", text("rs_a"))]))]
fn skips_a_non_list_output(#[case] non_list: Record) {
    let mut response = dict(vec![("output", non_list)]);
    let original = response.clone();
    encode_encrypted_item_ids(&mut response, Some("dep_a"));
    assert_eq!(response, original);
}

#[rstest]
#[case::empty_string(Some(text("")))]
#[case::number(Some(number(7)))]
#[case::null(Some(null()))]
#[case::absent(None)]
fn skips_empty_or_non_string_content(#[case] content: Option<Record>) {
    let mut item = vec![("id", text("rs_a"))];
    if let Some(found) = content {
        item.push(("encrypted_content", found));
    }
    let mut response = output(vec![dict(item)]);
    let original = response.clone();
    encode_encrypted_item_ids(&mut response, Some("dep_a"));
    assert_eq!(response, original);
}

#[test]
fn skips_an_empty_id_but_still_wraps_the_content() {
    let mut response = output(vec![dict(vec![
        ("id", text("")),
        ("encrypted_content", text("cipher")),
    ])]);
    encode_encrypted_item_ids(&mut response, Some("dep_a"));
    assert_eq!(
        item_content(&response, 0),
        Some(wrap_encrypted_content("cipher", "dep_a"))
    );
    assert_eq!(item_id(&response, 0), Some(String::new()));
}

#[test]
fn swallows_a_read_only_content_write_and_still_encodes_the_id() {
    let mut response = output(vec![object(
        vec![("id", text("rs_a")), ("encrypted_content", text("cipher"))],
        &["encrypted_content"],
    )]);
    let ((), logs) = capture(|| encode_encrypted_item_ids(&mut response, Some("dep_a")));
    assert_eq!(item_content(&response, 0), Some("cipher".to_owned()));
    assert_eq!(
        item_id(&response, 0),
        Some(build_encrypted_item_id("dep_a", "rs_a"))
    );
    assert!(logs.is_empty());
}

#[test]
fn swallows_a_read_only_id_write_and_still_wraps_the_content() {
    let mut response = output(vec![object(
        vec![("id", text("rs_a")), ("encrypted_content", text("cipher"))],
        &["id"],
    )]);
    let ((), logs) = capture(|| encode_encrypted_item_ids(&mut response, Some("dep_a")));
    assert_eq!(
        item_content(&response, 0),
        Some(wrap_encrypted_content("cipher", "dep_a"))
    );
    assert_eq!(item_id(&response, 0), Some("rs_a".to_owned()));
    assert!(logs.is_empty());
}

#[test]
fn leaves_scalar_items_alone() {
    let mut response = output(vec![text("plain"), list(vec![text("inner")])]);
    let original = response.clone();
    encode_encrypted_item_ids(&mut response, Some("dep_a"));
    assert_eq!(response, original);
}
