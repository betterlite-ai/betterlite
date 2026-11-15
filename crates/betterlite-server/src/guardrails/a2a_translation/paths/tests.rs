#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{TextPlacement, apply_text_to_path, extract_texts_from_result};

fn result(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

fn placement(path: &[&str], part_index: usize, text: &str) -> TextPlacement {
    TextPlacement {
        path: path.iter().map(|key| (*key).to_owned()).collect(),
        part_index,
        text: text.to_owned(),
    }
}

#[test]
fn paths_collect_the_five_locations_in_order() {
    let value = result(
        r#"{"parts":[{"kind":"text","text":"direct"}],
        "message":{"parts":[{"kind":"text","text":"nested"}]},
        "artifact":{"parts":[{"kind":"text","text":"artifact"}]},
        "status":{"message":{"parts":[{"kind":"text","text":"status"}]}},
        "artifacts":[{"parts":[{"kind":"text","text":"first"}]},{"parts":[{"kind":"text","text":"second"}]}]}"#,
    );
    assert_eq!(
        extract_texts_from_result(&value),
        vec![
            placement(&["parts"], 0, "direct"),
            placement(&["message", "parts"], 0, "nested"),
            placement(&["artifact", "parts"], 0, "artifact"),
            placement(&["status", "message", "parts"], 0, "status"),
            placement(&["artifacts", "0", "parts"], 0, "first"),
            placement(&["artifacts", "1", "parts"], 0, "second"),
        ]
    );
}

#[rstest]
#[case::direct(r#"{"parts":[{"kind":"text","text":"x"}]}"#, &["parts"])]
#[case::message(r#"{"message":{"parts":[{"kind":"text","text":"x"}]}}"#, &["message", "parts"])]
#[case::artifact(r#"{"artifact":{"parts":[{"kind":"text","text":"x"}]}}"#, &["artifact", "parts"])]
#[case::status(r#"{"status":{"message":{"parts":[{"kind":"text","text":"x"}]}}}"#, &["status", "message", "parts"])]
#[case::artifacts(r#"{"artifacts":[{"parts":[{"kind":"text","text":"x"}]}]}"#, &["artifacts", "0", "parts"])]
#[test]
fn paths_collect_each_location_alone(#[case] raw: &str, #[case] path: &[&str]) {
    assert_eq!(
        extract_texts_from_result(&result(raw)),
        vec![placement(path, 0, "x")]
    );
}

#[rstest]
#[case::kind_not_text(r#"{"parts":[{"kind":"data","text":"x"}]}"#)]
#[case::kind_absent(r#"{"parts":[{"text":"x"}]}"#)]
#[case::kind_not_a_string(r#"{"parts":[{"kind":7,"text":"x"}]}"#)]
#[case::text_absent(r#"{"parts":[{"kind":"text"}]}"#)]
#[case::text_null(r#"{"parts":[{"kind":"text","text":null}]}"#)]
#[case::text_not_a_string(r#"{"parts":[{"kind":"text","text":7}]}"#)]
#[case::text_empty(r#"{"parts":[{"kind":"text","text":""}]}"#)]
#[case::part_not_an_object(r#"{"parts":[7,"x",[]]}"#)]
#[case::parts_null(r#"{"parts":null}"#)]
#[case::parts_object(r#"{"parts":{"kind":"text","text":"x"}}"#)]
#[case::artifacts_not_an_array(r#"{"artifacts":"x"}"#)]
#[case::artifact_item_without_parts(r#"{"artifacts":[{"kind":"artifact"}]}"#)]
#[case::artifact_item_not_an_object(r#"{"artifacts":[7]}"#)]
#[test]
fn paths_skip_what_is_not_a_text_part(#[case] raw: &str) {
    assert!(extract_texts_from_result(&result(raw)).is_empty());
}

#[test]
fn paths_keep_the_part_index_of_each_text_part() {
    let value = result(
        r#"{"parts":[{"kind":"data"},{"kind":"text","text":"a"},7,{"kind":"text","text":"b"}]}"#,
    );
    assert_eq!(
        extract_texts_from_result(&value),
        vec![placement(&["parts"], 1, "a"), placement(&["parts"], 3, "b"),]
    );
}

#[test]
fn paths_write_through_a_decimal_index_into_the_second_artifact() {
    let mut value = result(
        r#"{"artifacts":[{"parts":[{"kind":"text","text":"a"}]},{"parts":[{"kind":"text","text":"b"}]}]}"#,
    );
    apply_text_to_path(
        &mut value,
        &["artifacts".to_owned(), "1".to_owned(), "parts".to_owned()],
        0,
        "new",
    );
    assert_eq!(
        extract_texts_from_result(&value),
        vec![
            placement(&["artifacts", "0", "parts"], 0, "a"),
            placement(&["artifacts", "1", "parts"], 0, "new"),
        ]
    );
}

#[test]
fn paths_write_replaces_only_the_named_part() {
    let mut value = result(
        r#"{"message":{"parts":[{"kind":"text","text":"a"},{"kind":"data"},{"kind":"text","text":"b"}]}}"#,
    );
    let path = ["message".to_owned(), "parts".to_owned()];
    apply_text_to_path(&mut value, &path, 2, "new");
    assert_eq!(
        extract_texts_from_result(&value),
        vec![
            placement(&["message", "parts"], 0, "a"),
            placement(&["message", "parts"], 2, "new"),
        ]
    );
}

#[rstest]
#[case::empty(&[])]
#[case::missing_key(&["missing"])]
#[case::wrong_container(&["parts", "0"])]
#[case::index_out_of_bounds(&["artifacts", "9", "parts"])]
#[test]
fn paths_write_leaves_the_result_alone_when_the_path_does_not_fit(#[case] path: &[&str]) {
    let mut value = result(
        r#"{"parts":[{"kind":"text","text":"a"}],"artifacts":[{"parts":[{"kind":"text","text":"b"}]}]}"#,
    );
    let untouched = value.clone();
    let owned_path = path.iter().map(|key| (*key).to_owned()).collect::<Vec<_>>();
    apply_text_to_path(&mut value, &owned_path, 0, "new");
    assert_eq!(value, untouched);
}
