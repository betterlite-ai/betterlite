use rstest::rstest;

use crate::api::response_id_codecs::container::build_container_id;
use crate::api::response_id_traversal::container::collect_container_ids;
use crate::api::response_id_traversal::record::Record;

use super::fixtures::{dict, list, null, number, object, output, text};

#[test]
fn dedupes_ids_from_dicts_and_objects() {
    let response = output(vec![
        dict(vec![("container_id", text("c1"))]),
        object(vec![("container_id", text("c1"))], &[]),
        dict(vec![(
            "code_interpreter_call",
            dict(vec![("container_id", text("c2"))]),
        )]),
        object(
            vec![(
                "code_interpreter_call",
                object(vec![("container_id", text("c2"))], &[]),
            )],
            &[],
        ),
        dict(vec![
            ("type", text("message")),
            (
                "content",
                list(vec![dict(vec![(
                    "annotations",
                    list(vec![
                        dict(vec![("container_id", text("c3"))]),
                        dict(vec![("container_id", text("c1"))]),
                    ]),
                )])]),
            ),
        ]),
    ]);
    assert_eq!(
        collect_container_ids(Some(&response)),
        vec!["c1".to_owned(), "c2".to_owned(), "c3".to_owned()]
    );
}

#[test]
fn reads_object_message_content() {
    let response = output(vec![object(
        vec![
            ("type", text("message")),
            (
                "content",
                list(vec![object(
                    vec![(
                        "annotations",
                        list(vec![object(vec![("container_id", text("c9"))], &[])]),
                    )],
                    &[],
                )]),
            ),
        ],
        &[],
    )]);
    assert_eq!(
        collect_container_ids(Some(&response)),
        vec!["c9".to_owned()]
    );
}

#[rstest]
#[case::empty(Some(text("")), 0)]
#[case::null(Some(null()), 0)]
#[case::number(Some(number(7)), 0)]
#[case::missing(None, 0)]
#[case::kept(Some(text("c1")), 1)]
fn ignores_non_string_ids(#[case] container_id: Option<Record>, #[case] found: usize) {
    let mut entries = Vec::new();
    if let Some(value) = container_id {
        entries.push(("container_id", value));
    }
    let response = output(vec![dict(entries)]);
    assert_eq!(collect_container_ids(Some(&response)).len(), found);
}

#[rstest]
#[case::no_response(None)]
#[case::no_output(Some(dict(vec![])))]
#[case::text_output(Some(dict(vec![("output", text("x"))])))]
#[case::mapping_output(Some(dict(vec![("output", dict(vec![("container_id", text("c1"))]))])))]
fn returns_no_ids_without_a_list_output(#[case] response: Option<Record>) {
    assert_eq!(
        collect_container_ids(response.as_ref()),
        Vec::<String>::new()
    );
}

#[test]
fn neither_decodes_nor_mutates() {
    let managed = build_container_id(Some("other"), Some("m"), "c");
    let raw = dict(vec![
        ("container_id", text(&managed)),
        (
            "code_interpreter_call",
            dict(vec![("container_id", text("c2"))]),
        ),
    ]);
    let response = output(vec![raw]);
    let original = response.clone();
    assert_eq!(
        collect_container_ids(Some(&response)),
        vec![managed, "c2".to_owned()]
    );
    assert_eq!(response, original);
}

#[test]
fn ignores_annotations_outside_messages() {
    let response = output(vec![dict(vec![
        ("type", text("function_call")),
        (
            "content",
            list(vec![dict(vec![(
                "annotations",
                list(vec![dict(vec![("container_id", text("c3"))])]),
            )])]),
        ),
    ])]);
    assert_eq!(collect_container_ids(Some(&response)), Vec::<String>::new());
}

#[test]
fn ignores_non_list_containers() {
    let response = output(vec![
        dict(vec![("type", text("message")), ("content", text("x"))]),
        dict(vec![
            ("type", text("message")),
            (
                "content",
                list(vec![dict(vec![("annotations", text("x"))])]),
            ),
        ]),
    ]);
    assert_eq!(collect_container_ids(Some(&response)), Vec::<String>::new());
}

#[test]
fn reads_a_mapping_nested_in_an_object() {
    let response = output(vec![object(
        vec![(
            "code_interpreter_call",
            dict(vec![("container_id", text("c2"))]),
        )],
        &[],
    )]);
    assert_eq!(
        collect_container_ids(Some(&response)),
        vec!["c2".to_owned()]
    );
}

#[test]
fn does_not_descend_into_an_object_nested_in_a_mapping() {
    let response = output(vec![dict(vec![(
        "code_interpreter_call",
        object(vec![("container_id", text("c2"))], &[]),
    )])]);
    assert_eq!(collect_container_ids(Some(&response)), Vec::<String>::new());
}

#[test]
fn collects_only_the_one_nested_mapping_id() {
    let inner = dict(vec![("container_id", text("c9"))]);
    let nested = dict(vec![
        ("container_id", text("c2")),
        ("code_interpreter_call", inner),
    ]);
    let response = output(vec![dict(vec![("code_interpreter_call", nested)])]);
    assert_eq!(
        collect_container_ids(Some(&response)),
        vec!["c2".to_owned()]
    );
}

#[test]
fn ignores_a_nested_mappings_message_annotations() {
    let part = dict(vec![(
        "annotations",
        list(vec![dict(vec![("container_id", text("c3"))])]),
    )]);
    let nested = dict(vec![
        ("type", text("message")),
        ("content", list(vec![part])),
    ]);
    let response = output(vec![dict(vec![("code_interpreter_call", nested)])]);
    assert_eq!(collect_container_ids(Some(&response)), Vec::<String>::new());
}

#[test]
fn reads_a_mapping_nested_in_an_object_through_its_message_content() {
    let part = dict(vec![(
        "annotations",
        list(vec![dict(vec![("container_id", text("c3"))])]),
    )]);
    let nested = dict(vec![
        ("type", text("message")),
        ("content", list(vec![part])),
    ]);
    let response = output(vec![object(vec![("code_interpreter_call", nested)], &[])]);
    assert_eq!(
        collect_container_ids(Some(&response)),
        vec!["c3".to_owned()]
    );
}

#[test]
fn ignores_scalar_items() {
    let response = output(vec![text("plain"), list(vec![text("c1")])]);
    assert_eq!(collect_container_ids(Some(&response)), Vec::<String>::new());
}
