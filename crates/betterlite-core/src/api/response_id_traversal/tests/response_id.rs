use rstest::rstest;

use crate::api::response_id_codecs::container::build_container_id;
use crate::api::response_id_codecs::encrypted::wrap_encrypted_content;
use crate::api::response_id_codecs::response::build_response_id;
use crate::api::response_id_traversal::metadata::{AffinityMetadata, ModelInfo};
use crate::api::response_id_traversal::record::{Record, TraversalError};
use crate::api::response_id_traversal::response_id::rewrite_response_id;

use super::fixtures::{dict, list, object, output, text, text_field};

fn metadata(model_id: Option<&str>) -> AffinityMetadata {
    AffinityMetadata {
        model_info: Some(ModelInfo {
            id: model_id.map(str::to_owned),
        }),
        affinity_enabled: false,
    }
}

fn raw_output() -> Record {
    output(vec![
        dict(vec![
            ("id", text("rs_a")),
            ("encrypted_content", text("cipher")),
            ("container_id", text("c1")),
        ]),
        dict(vec![(
            "code_interpreter_call",
            dict(vec![("container_id", text("c2"))]),
        )]),
    ])
}

#[test]
fn rewrites_the_id_with_provider_and_model() {
    let mut response = dict(vec![
        ("id", text("resp_a")),
        (
            "output",
            list(vec![dict(vec![("container_id", text("c1"))])]),
        ),
    ]);
    rewrite_response_id(&mut response, Some("openai"), &metadata(Some("dep_a"))).unwrap();
    let expected = build_response_id(Some("openai"), Some("dep_a"), "resp_a");
    assert_eq!(text_field(&response, "id"), Some(expected));
}

#[rstest]
#[case::default_metadata(AffinityMetadata::default())]
#[case::empty_model_info(AffinityMetadata {
    model_info: Some(ModelInfo { id: None }),
    affinity_enabled: false,
})]
fn missing_model_encodes_as_none(#[case] metadata: AffinityMetadata) {
    let mut response = dict(vec![("id", text("resp_a"))]);
    rewrite_response_id(&mut response, Some("openai"), &metadata).unwrap();
    let expected = build_response_id(Some("openai"), None, "resp_a");
    assert_eq!(text_field(&response, "id"), Some(expected));
}

#[test]
fn absent_provider_encodes_as_none() {
    let mut response = dict(vec![("id", text("resp_a"))]);
    rewrite_response_id(&mut response, None, &metadata(Some("dep_a"))).unwrap();
    let expected = build_response_id(None, Some("dep_a"), "resp_a");
    assert_eq!(text_field(&response, "id"), Some(expected));
}

#[test]
fn empty_model_id_encodes_as_empty() {
    let mut response = dict(vec![("id", text("resp_a"))]);
    rewrite_response_id(&mut response, Some("openai"), &metadata(Some(""))).unwrap();
    let expected = build_response_id(Some("openai"), Some(""), "resp_a");
    assert_eq!(text_field(&response, "id"), Some(expected));
}

#[test]
fn empty_id_is_encoded() {
    let mut response = dict(vec![("id", text(""))]);
    rewrite_response_id(&mut response, Some("openai"), &metadata(Some("dep_a"))).unwrap();
    let expected = build_response_id(Some("openai"), Some("dep_a"), "");
    assert_eq!(text_field(&response, "id"), Some(expected));
}

#[test]
fn missing_id_leaves_nested_fields_untouched() {
    let mut response = raw_output();
    let original = response.clone();
    let mut settings = metadata(Some("dep_a"));
    settings.affinity_enabled = true;
    rewrite_response_id(&mut response, Some("openai"), &settings).unwrap();
    assert_eq!(response, original);
}

#[test]
fn managed_id_leaves_nested_fields_untouched() {
    let managed = build_response_id(Some("openai"), Some("dep_a"), "resp_a");
    let mut response = raw_output();
    if let Some(slot) = response.field_mut("id") {
        *slot = text(&managed);
    }
    let original = response.clone();
    let mut settings = metadata(Some("dep_a"));
    settings.affinity_enabled = true;
    rewrite_response_id(&mut response, Some("openai"), &settings).unwrap();
    assert_eq!(response, original);
}

#[test]
fn affinity_flag_gates_encrypted_rewrites_only() {
    let mut response = dict(vec![
        ("id", text("resp_a")),
        (
            "output",
            list(vec![dict(vec![
                ("id", text("rs_a")),
                ("encrypted_content", text("cipher")),
                ("container_id", text("c1")),
            ])]),
        ),
    ]);
    rewrite_response_id(&mut response, Some("openai"), &metadata(Some("dep_a"))).unwrap();
    let Some(Record::List(items)) = response.field("output") else {
        panic!("output should stay a list");
    };
    let Some(item) = items.first() else {
        panic!("the output item should stay");
    };
    assert_eq!(
        text_field(item, "encrypted_content"),
        Some("cipher".to_owned())
    );
    assert_eq!(
        text_field(item, "container_id"),
        Some(build_container_id(Some("openai"), Some("dep_a"), "c1"))
    );
}

#[test]
fn affinity_flag_runs_encrypted_rewrites() {
    let mut response = dict(vec![
        ("id", text("resp_a")),
        (
            "output",
            list(vec![dict(vec![
                ("id", text("rs_a")),
                ("encrypted_content", text("cipher")),
            ])]),
        ),
    ]);
    let mut settings = metadata(Some("dep_a"));
    settings.affinity_enabled = true;
    rewrite_response_id(&mut response, Some("openai"), &settings).unwrap();
    let Some(Record::List(items)) = response.field("output") else {
        panic!("output should stay a list");
    };
    assert_eq!(
        items
            .first()
            .and_then(|item| item.field("encrypted_content").and_then(Record::as_text)),
        Some(wrap_encrypted_content("cipher", "dep_a").as_str())
    );
}

#[test]
fn read_only_id_write_is_an_error_before_any_traversal() {
    let mut response = object(
        vec![
            ("id", text("resp_a")),
            (
                "output",
                list(vec![dict(vec![("container_id", text("c1"))])]),
            ),
        ],
        &["id"],
    );
    let original = response.clone();
    let error = rewrite_response_id(&mut response, Some("openai"), &metadata(Some("dep_a")))
        .expect_err("a read-only id refuses the write");
    assert_eq!(
        error,
        TraversalError::ReadOnly {
            field: "id".to_owned()
        }
    );
    assert_eq!(response, original);
}
