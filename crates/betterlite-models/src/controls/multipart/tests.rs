#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::controls::json_value::JsonValue;
use crate::controls::multipart::{
    FormField, FormFieldError, FormValue, flatten_form_field_values, serialize_multipart_fields,
};

enum Action {
    Flatten {
        sources: Vec<JsonValue>,
        max_depth: usize,
    },
    Serialize {
        data: JsonValue,
        max_depth: usize,
    },
}

enum Expected {
    Form(Vec<(&'static str, FormValue)>),
    Multipart(Vec<(&'static str, &'static str)>),
    Error(FormFieldError),
}

fn object(entries: Vec<(&str, JsonValue)>) -> JsonValue {
    let mut fields = IndexMap::new();
    for (name, value) in entries {
        fields.insert(name.to_owned(), value);
    }
    JsonValue::Object(fields)
}

fn array(items: Vec<JsonValue>) -> JsonValue {
    JsonValue::Array(items)
}

fn integer(value: i64) -> JsonValue {
    JsonValue::Number(Number::from(value))
}

fn text(value: &str) -> JsonValue {
    JsonValue::Text(value.to_owned())
}

fn nested_arrays(depth: usize) -> JsonValue {
    let mut value = integer(1);
    for _ in 0..depth {
        value = array(vec![value]);
    }
    value
}

fn flatten_source(sources: &[JsonValue], max_depth: usize) -> Vec<FormField> {
    let borrowed: Vec<Option<&JsonValue>> = sources.iter().map(Some).collect();
    flatten_form_field_values(&borrowed, max_depth).expect("the form fields flatten")
}

fn varied_scalars() -> JsonValue {
    object(vec![
        ("a", integer(1)),
        ("b", JsonValue::Null),
        ("c", text("")),
        ("d", JsonValue::Bool(false)),
        ("e", JsonValue::Bool(true)),
    ])
}

#[rstest]
#[case::scalars_are_serialized(
    Action::Flatten { sources: vec![varied_scalars()], max_depth: 100 },
    Expected::Form(vec![
        ("a", FormValue::Single("1".to_owned())),
        ("d", FormValue::Single("false".to_owned())),
        ("e", FormValue::Single("true".to_owned())),
    ])
)]
#[case::a_scalar_list_stays_one_field(
    Action::Flatten { sources: vec![object(vec![("list", array(vec![integer(1), integer(2), integer(3)]))])], max_depth: 100 },
    Expected::Form(vec![("list", FormValue::Repeated(vec!["1".to_owned(), "2".to_owned(), "3".to_owned()]))])
)]
#[case::a_mixed_list_expands_and_drops_empty_entries(
    Action::Flatten { sources: vec![object(vec![("list", array(vec![integer(1), text(""), JsonValue::Null]))])], max_depth: 100 },
    Expected::Form(vec![("list[]", FormValue::Single("1".to_owned()))])
)]
#[case::a_list_of_lists_repeats_each_inner_value(
    Action::Flatten { sources: vec![object(vec![("list", array(vec![array(vec![integer(1)]), array(vec![integer(2)])]))])], max_depth: 100 },
    Expected::Form(vec![
        ("list[]", FormValue::Repeated(vec!["1".to_owned()])),
        ("list[]", FormValue::Repeated(vec!["2".to_owned()])),
    ])
)]
#[case::nested_objects_and_lists_are_named(
    Action::Flatten { sources: vec![object(vec![("nested", object(vec![("x", object(vec![("y", integer(1))])), ("z", array(vec![integer(1), integer(2)]))]))])], max_depth: 100 },
    Expected::Form(vec![
        ("nested[x][y]", FormValue::Single("1".to_owned())),
        ("nested[z]", FormValue::Repeated(vec!["1".to_owned(), "2".to_owned()])),
    ])
)]
#[case::empty_containers_produce_no_field(
    Action::Flatten { sources: vec![object(vec![("empty", object(vec![])), ("empty_list", array(vec![]))])], max_depth: 100 },
    Expected::Form(vec![])
)]
#[case::a_list_of_empty_strings_produces_no_field(
    Action::Flatten { sources: vec![object(vec![("list", array(vec![text(""), text("")]))])], max_depth: 100 },
    Expected::Form(vec![])
)]
#[case::later_sources_append_their_fields(
    Action::Flatten { sources: vec![object(vec![("first", integer(1))]), object(vec![("second", integer(2))])], max_depth: 100 },
    Expected::Form(vec![
        ("first", FormValue::Single("1".to_owned())),
        ("second", FormValue::Single("2".to_owned())),
    ])
)]
#[case::a_scalar_list_at_the_depth_limit_is_allowed(
    Action::Flatten { sources: vec![object(vec![("list", array(vec![integer(1)]))])], max_depth: 0 },
    Expected::Form(vec![("list", FormValue::Repeated(vec!["1".to_owned()]))])
)]
#[case::a_nested_list_past_the_depth_limit_fails(
    Action::Flatten { sources: vec![object(vec![("list", nested_arrays(2))])], max_depth: 0 },
    Expected::Error(FormFieldError::TooDeep { limit: 0 })
)]
#[case::a_source_that_is_not_an_object_fails(
    Action::Flatten { sources: vec![array(vec![])], max_depth: 100 },
    Expected::Error(FormFieldError::NotAnObject)
)]
#[case::multipart_expands_every_list_element(
    Action::Serialize { data: object(vec![("list", array(vec![integer(1), integer(2), integer(3)]))]), max_depth: 100 },
    Expected::Multipart(vec![("list[]", "1"), ("list[]", "2"), ("list[]", "3")])
)]
#[case::multipart_names_nested_objects_and_lists(
    Action::Serialize { data: object(vec![("nested", object(vec![("x", object(vec![("y", integer(1))])), ("z", array(vec![integer(1), integer(2)]))]))]), max_depth: 100 },
    Expected::Multipart(vec![("nested[x][y]", "1"), ("nested[z][]", "1"), ("nested[z][]", "2")])
)]
#[case::multipart_drops_null_and_empty_values(
    Action::Serialize { data: object(vec![("a", JsonValue::Null), ("b", text("")), ("c", integer(0))]), max_depth: 100 },
    Expected::Multipart(vec![("c", "0")])
)]
#[case::multipart_at_the_depth_limit_is_allowed(
    Action::Serialize { data: object(vec![("a", object(vec![("b", integer(1))]))]), max_depth: 1 },
    Expected::Multipart(vec![("a[b]", "1")])
)]
#[case::multipart_past_the_depth_limit_fails(
    Action::Serialize { data: object(vec![("a", object(vec![("b", integer(1))]))]), max_depth: 0 },
    Expected::Error(FormFieldError::TooDeep { limit: 0 })
)]
#[case::multipart_data_that_is_not_an_object_fails(
    Action::Serialize { data: array(vec![]), max_depth: 100 },
    Expected::Error(FormFieldError::NotAnObject)
)]
fn provider_controls_tests_behavior(#[case] action: Action, #[case] expected: Expected) {
    match (action, expected) {
        (Action::Flatten { sources, max_depth }, Expected::Form(expected_fields)) => {
            let flattened = flatten_source(&sources, max_depth);
            let rendered: Vec<(&str, FormValue)> = flattened
                .iter()
                .map(|field| (field.name.as_str(), field.value.clone()))
                .collect();
            assert_eq!(rendered, expected_fields);
        }
        (Action::Flatten { sources, max_depth }, Expected::Error(expected_error)) => {
            let borrowed: Vec<Option<&JsonValue>> = sources.iter().map(Some).collect();
            assert_eq!(
                flatten_form_field_values(&borrowed, max_depth),
                Err(expected_error)
            );
        }
        (Action::Serialize { data, max_depth }, Expected::Multipart(expected_fields)) => {
            let serialized =
                serialize_multipart_fields(&data, max_depth).expect("the multipart fields flatten");
            let rendered: Vec<(&str, &str)> = serialized
                .iter()
                .map(|field| (field.name.as_str(), field.value.as_str()))
                .collect();
            assert_eq!(rendered, expected_fields);
        }
        (Action::Serialize { data, max_depth }, Expected::Error(expected_error)) => {
            assert_eq!(
                serialize_multipart_fields(&data, max_depth).map(|fields| fields.len()),
                Err(expected_error)
            );
        }
        (Action::Flatten { .. }, Expected::Multipart(_))
        | (Action::Serialize { .. }, Expected::Form(_)) => {
            panic!("the expectation does not match the action")
        }
    }
}
