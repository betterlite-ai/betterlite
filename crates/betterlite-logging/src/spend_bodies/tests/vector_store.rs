use super::support::value;
use crate::python_json;
use crate::spend_bodies::vector_store::redact_vector_store;
use crate::spend_sanitize::REDACTED_BY_LITELM;
use crate::spend_sanitize::value::{NodeId, SpendList, SpendValue, to_json};

fn requests(text: &str) -> Vec<SpendValue> {
    let SpendValue::List(list) = value(text) else {
        panic!("fixture must be a JSON list");
    };
    list.items
}

fn stored(requests: &mut [SpendValue], store_bodies: bool) -> String {
    redact_vector_store(requests, store_bodies);
    let list = SpendValue::List(SpendList {
        node: NodeId::from(90_u64),
        items: requests.to_vec(),
    });
    python_json::dumps(&to_json(&list))
}

fn untouched(text: &str) -> String {
    python_json::dumps(&to_json(&SpendValue::List(SpendList {
        node: NodeId::from(90_u64),
        items: requests(text),
    })))
}

#[test]
fn redacts_content_text_when_body_storage_is_disabled() {
    let mut entries = requests(
        r#"[{"vector_store_search_response": {"data": [{"score": 0.9, "content": [{"type": "text", "text": "secret", "metadata": {"page": 2}}, {"type": "image", "url": "u"}]}]}}]"#,
    );
    let expected = format!(
        r#"[{{"vector_store_search_response": {{"data": [{{"score": 0.9, "content": [{{"type": "text", "text": "{REDACTED_BY_LITELM}", "metadata": {{"page": 2}}}}, {{"type": "image", "url": "u"}}]}}]}}}}]"#
    );
    assert_eq!(stored(&mut entries, false), expected);
}

#[test]
fn keeps_content_when_body_storage_is_enabled() {
    let source =
        r#"[{"vector_store_search_response": {"data": [{"content": [{"text": "secret"}]}]}}]"#;
    let mut entries = requests(source);
    assert_eq!(stored(&mut entries, true), untouched(source));
}

#[rstest::rstest]
#[case::missing_response(r#"[{"other": 1}]"#)]
#[case::null_response(r#"[{"vector_store_search_response": null}]"#)]
#[case::missing_data(r#"[{"vector_store_search_response": {}}]"#)]
#[case::non_list_data(r#"[{"vector_store_search_response": {"data": "text"}}]"#)]
#[case::non_list_content(r#"[{"vector_store_search_response": {"data": [{"content": "text"}]}}]"#)]
#[case::non_mapping_items(r#"[{"vector_store_search_response": {"data": [1, "x", null]}}]"#)]
fn tolerates_missing_or_malformed_shapes(#[case] source: &str) {
    let mut entries = requests(source);
    assert_eq!(stored(&mut entries, false), untouched(source));
}
