use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;

use crate::api::response_id_traversal::reasoning::strip_encrypted_reasoning;
use crate::api::response_id_traversal::record::Record;

use super::fixtures::{dict, list, null, number, object, text};

fn reasoning(summary: Record) -> Record {
    dict(vec![
        ("type", text("reasoning")),
        ("id", text("r")),
        ("encrypted_content", text("cipher")),
        ("summary", summary),
    ])
}

fn stripped_reasoning(summary: &Record) -> Record {
    dict(vec![
        ("type", text("reasoning")),
        ("summary", summary.clone()),
    ])
}

#[test]
fn drops_unreadable_reasoning() {
    let mut input = list(vec![reasoning(list(Vec::new()))]);
    strip_encrypted_reasoning(&mut input, None);
    assert_eq!(input, list(Vec::new()));
}

#[test]
fn keeps_a_readable_content_string() {
    let item = dict(vec![
        ("type", text("reasoning")),
        ("id", text("r")),
        ("encrypted_content", text("cipher")),
        ("content", text("visible")),
    ]);
    let mut input = list(vec![item.clone()]);
    strip_encrypted_reasoning(&mut input, None);
    let Record::Dict(mut kept) = item else {
        panic!("the item should be a mapping");
    };
    kept.values.shift_remove("id");
    kept.values.shift_remove("encrypted_content");
    assert_eq!(input, list(vec![Record::Dict(kept)]));
}

#[rstest]
#[case::whitespace_block(list(vec![dict(vec![("text", text(" "))])]), true)]
#[case::empty_block(list(vec![dict(vec![("text", text(""))])]), false)]
#[case::zero_block(list(vec![dict(vec![("text", number(0))])]), false)]
#[case::one_block(list(vec![dict(vec![("text", number(1))])]), true)]
#[case::missing_text(list(vec![dict(vec![("type", text("summary_text"))])]), false)]
#[case::empty_list(list(Vec::new()), false)]
#[case::string_block(list(vec![text("block")]), false)]
#[case::object_block(list(vec![object(vec![("text", text(" "))], &[])]), false)]
#[case::whitespace_string(text(" "), false)]
#[case::visible_string(text("visible"), true)]
fn summary_readability(#[case] summary: Record, #[case] readable: bool) {
    let summary_value = summary.clone();
    let mut input = list(vec![reasoning(summary)]);
    strip_encrypted_reasoning(&mut input, None);
    let expected = if readable {
        list(vec![stripped_reasoning(&summary_value)])
    } else {
        list(Vec::new())
    };
    assert_eq!(input, expected);
}

#[rstest]
#[case::non_reasoning_type(dict(vec![("type", text("message")), ("encrypted_content", text("cipher"))]))]
#[case::missing_type(dict(vec![("encrypted_content", text("cipher"))]))]
#[case::empty_cipher(dict(vec![("type", text("reasoning")), ("encrypted_content", text(""))]))]
#[case::empty_mapping_cipher(dict(vec![("type", text("reasoning")), ("encrypted_content", dict(vec![]))]))]
#[case::empty_list_cipher(dict(vec![("type", text("reasoning")), ("encrypted_content", list(vec![]))]))]
#[case::null_cipher(dict(vec![("type", text("reasoning")), ("encrypted_content", null())]))]
#[case::missing_cipher(dict(vec![("type", text("reasoning"))]))]
fn keeps_items_that_are_not_encrypted_reasoning(#[case] item: Record) {
    let mut input = list(vec![item.clone()]);
    strip_encrypted_reasoning(&mut input, None);
    assert_eq!(input, list(vec![item]));
}

#[test]
fn a_predicate_selects_items_only_when_it_is_given() {
    let selected = dict(vec![
        ("type", text("reasoning")),
        ("id", text("r")),
        ("encrypted_content", text("cipher")),
        ("summary", text("source")),
    ]);
    let other = dict(vec![
        ("type", text("reasoning")),
        ("id", text("s")),
        ("encrypted_content", text("cipher")),
        ("summary", text("source")),
    ]);
    let mut input = list(vec![selected, other.clone()]);
    let predicate = |item: &Record| item.field("id").and_then(Record::as_text) == Some("r");
    strip_encrypted_reasoning(&mut input, Some(&predicate));
    assert_eq!(
        input,
        list(vec![stripped_reasoning(&text("source")), other])
    );
}

#[test]
fn a_predicate_never_sees_non_dictionary_items() {
    let calls = AtomicUsize::new(0);
    let predicate = |item: &Record| {
        calls.fetch_add(1, Ordering::Relaxed);
        item.field("id").and_then(Record::as_text) == Some("r")
    };
    let object_item = object(
        vec![
            ("type", text("reasoning")),
            ("id", text("r")),
            ("encrypted_content", text("cipher")),
        ],
        &[],
    );
    let mut input = list(vec![object_item.clone()]);
    strip_encrypted_reasoning(&mut input, Some(&predicate));
    assert_eq!(input, list(vec![object_item]));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn drops_null_items() {
    let mut input = list(vec![null(), dict(vec![("type", text("message"))]), null()]);
    strip_encrypted_reasoning(&mut input, None);
    assert_eq!(input, list(vec![dict(vec![("type", text("message"))])]));
}

#[test]
fn keeps_the_surviving_order() {
    let mut input = list(vec![
        dict(vec![("type", text("first"))]),
        reasoning(list(Vec::new())),
        dict(vec![("type", text("third"))]),
    ]);
    strip_encrypted_reasoning(&mut input, None);
    assert_eq!(
        input,
        list(vec![
            dict(vec![("type", text("first"))]),
            dict(vec![("type", text("third"))]),
        ])
    );
}

#[rstest]
#[case::mapping(dict(vec![("type", text("reasoning"))]))]
#[case::text(text("reasoning"))]
#[case::null(null())]
fn leaves_a_non_list_input_alone(#[case] mut input: Record) {
    let original = input.clone();
    strip_encrypted_reasoning(&mut input, None);
    assert_eq!(input, original);
}
