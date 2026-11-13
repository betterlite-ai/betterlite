use rstest::rstest;

use crate::api::response_id_codecs::container::build_container_id;
use crate::api::response_id_traversal::container::{encode_container_id, encode_containers};
use crate::api::response_id_traversal::metadata::{AffinityMetadata, ModelInfo};
use crate::api::response_id_traversal::record::Record;

use super::fixtures::{
    annotation_container_of, capture, container_of, dict, list, message_with_annotation,
    nested_call, nested_container_of, number, object, output, text,
};

const PROVIDER: Option<&str> = Some("openai");
const MODEL: Option<&str> = Some("dep_a");

fn settings() -> AffinityMetadata {
    AffinityMetadata {
        model_info: Some(ModelInfo {
            id: MODEL.map(str::to_owned),
        }),
        affinity_enabled: false,
    }
}

fn encoded(original: &str) -> String {
    build_container_id(PROVIDER, MODEL, original)
}

fn item_at(response: &Record, index: usize) -> Record {
    let Some(Record::List(items)) = response.field("output") else {
        panic!("output should stay a list");
    };
    items.get(index).cloned().unwrap_or(Record::Null)
}

#[test]
fn encodes_top_level_and_nested_ids() {
    let mut response = output(vec![
        dict(vec![("container_id", text("c1"))]),
        dict(vec![("code_interpreter_call", nested_call("c2", false))]),
    ]);
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(container_of(&item_at(&response, 0)), Some(encoded("c1")));
    assert_eq!(
        nested_container_of(&item_at(&response, 1)),
        Some(encoded("c2"))
    );
}

#[test]
fn encodes_message_annotation_ids() {
    let mut response = output(vec![message_with_annotation("c3", false)]);
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(
        annotation_container_of(&item_at(&response, 0)),
        Some(encoded("c3"))
    );
}

#[test]
fn ignores_annotations_outside_message_content() {
    let mut response = output(vec![dict(vec![
        ("type", text("function_call")),
        ("content", list(vec![message_with_annotation("c3", false)])),
    ])]);
    let original = response.clone();
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(response, original);
}

#[test]
fn keeps_managed_ids_that_carry_a_provider() {
    let managed = build_container_id(Some("other"), Some("m"), "c");
    let mut response = output(vec![dict(vec![("container_id", text(&managed))])]);
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(container_of(&item_at(&response, 0)), Some(managed));
}

#[test]
fn wraps_a_providerless_envelope_again() {
    let providerless = build_container_id(None, None, "c");
    let mut response = output(vec![dict(vec![("container_id", text(&providerless))])]);
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(
        container_of(&item_at(&response, 0)),
        Some(build_container_id(PROVIDER, MODEL, &providerless))
    );
}

#[rstest]
#[case::empty_string(Some(text("")), true)]
#[case::null(Some(Record::Null), false)]
#[case::number(Some(number(7)), false)]
#[case::absent(None, false)]
fn encodes_only_string_container_ids(
    #[case] container_id: Option<Record>,
    #[case] encoded_again: bool,
) {
    let mut entries = Vec::new();
    if let Some(found) = container_id {
        entries.push(("container_id", found));
    }
    let before = dict(entries.clone());
    let mut response = output(vec![dict(entries)]);
    encode_containers(&mut response, PROVIDER, &settings());
    if encoded_again {
        assert_eq!(container_of(&item_at(&response, 0)), Some(encoded("")));
    } else {
        assert_eq!(item_at(&response, 0), before);
    }
}

#[test]
fn encodes_object_items() {
    let mut response = output(vec![object(vec![("container_id", text("c1"))], &[])]);
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(container_of(&item_at(&response, 0)), Some(encoded("c1")));
}

#[test]
fn encodes_objects_nested_in_objects() {
    let mut response = output(vec![object(
        vec![("code_interpreter_call", nested_call("c2", true))],
        &[],
    )]);
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(
        nested_container_of(&item_at(&response, 0)),
        Some(encoded("c2"))
    );
}

#[test]
fn encodes_object_message_annotations() {
    let mut response = output(vec![message_with_annotation("c3", true)]);
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(
        annotation_container_of(&item_at(&response, 0)),
        Some(encoded("c3"))
    );
}

#[test]
fn does_not_descend_into_an_object_nested_in_a_mapping() {
    let mut response = output(vec![dict(vec![(
        "code_interpreter_call",
        nested_call("c2", true),
    )])]);
    let original = response.clone();
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(response, original);
}

#[test]
fn descends_into_a_mapping_nested_in_an_object() {
    let mut response = output(vec![object(
        vec![("code_interpreter_call", nested_call("c2", false))],
        &[],
    )]);
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(
        nested_container_of(&item_at(&response, 0)),
        Some(encoded("c2"))
    );
}

#[test]
fn logs_a_read_only_assignment_and_keeps_the_original_id() {
    let mut response = output(vec![object(
        vec![
            ("container_id", text("c1")),
            ("code_interpreter_call", nested_call("c2", true)),
        ],
        &["container_id"],
    )]);
    let ((), logs) = capture(|| encode_containers(&mut response, PROVIDER, &settings()));
    assert_eq!(container_of(&item_at(&response, 0)), Some("c1".to_owned()));
    assert_eq!(
        logs,
        vec!["Could not set container_id on streaming output item".to_owned()]
    );
}

#[test]
fn continues_past_a_read_only_assignment() {
    let mut response = output(vec![object(
        vec![
            ("container_id", text("c1")),
            ("type", text("message")),
            (
                "content",
                list(vec![dict(vec![(
                    "annotations",
                    list(vec![dict(vec![("container_id", text("c3"))])]),
                )])]),
            ),
            ("code_interpreter_call", nested_call("c2", true)),
        ],
        &["container_id"],
    )]);
    let ((), _logs) = capture(|| encode_containers(&mut response, PROVIDER, &settings()));
    assert_eq!(
        nested_container_of(&item_at(&response, 0)),
        Some(encoded("c2"))
    );
    assert_eq!(
        annotation_container_of(&item_at(&response, 0)),
        Some(encoded("c3"))
    );
}

#[rstest]
#[case::text_content(dict(vec![("type", text("message")), ("content", text("x"))]))]
#[case::object_part(dict(vec![
    ("type", text("message")),
    ("content", list(vec![object(vec![], &[])])),
]))]
#[case::text_annotations(dict(vec![
    ("type", text("message")),
    ("content", list(vec![dict(vec![("annotations", text("x"))])])),
]))]
#[case::scalar_annotation(dict(vec![
    ("type", text("message")),
    ("content", list(vec![dict(vec![("annotations", list(vec![text("ann")]))])])),
]))]
fn ignores_non_list_content_and_annotations(#[case] item: Record) {
    let mut response = output(vec![item]);
    let original = response.clone();
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(response, original);
}

#[rstest]
#[case::missing_output(dict(vec![]))]
#[case::text_output(dict(vec![("output", text("x"))]))]
#[case::mapping_output(dict(vec![("output", dict(vec![("container_id", text("c1"))]))]))]
fn does_nothing_without_a_list_output(#[case] mut response: Record) {
    let original = response.clone();
    encode_containers(&mut response, PROVIDER, &settings());
    assert_eq!(response, original);
}

#[test]
fn encodes_a_single_item_directly() {
    let mut item = dict(vec![("container_id", text("c1"))]);
    encode_container_id(&mut item, PROVIDER, MODEL);
    assert_eq!(container_of(&item), Some(encoded("c1")));
}
