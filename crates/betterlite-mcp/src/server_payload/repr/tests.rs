#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::{list, text, value};

#[test]
fn text_quotes_a_plain_string_with_single_quotes() {
    assert_eq!(text("oauth2"), "'oauth2'");
}

#[test]
fn text_uses_double_quotes_when_the_value_holds_a_single_quote() {
    assert_eq!(text("it's"), "\"it's\"");
}

#[test]
fn text_keeps_single_quotes_and_escapes_them_when_both_quotes_appear() {
    assert_eq!(text("it's \"this\""), "'it\\'s \"this\"'");
}

#[test]
fn text_escapes_backslash_newline_return_and_tab() {
    assert_eq!(text("a\\b\nc\rd\te"), "'a\\\\b\\nc\\rd\\te'");
}

#[test]
fn text_escapes_a_control_character() {
    assert_eq!(text("a\u{1}b"), "'a\\u0001b'");
}

#[test]
fn text_leaves_printable_unicode_alone() {
    assert_eq!(text("café"), "'café'");
}

#[test]
fn list_separates_string_reprs_with_a_comma_and_a_space() {
    let values = ["b".to_owned(), "a".to_owned()];
    assert_eq!(list(&values), "['b', 'a']");
}

#[test]
fn list_of_nothing_is_empty_brackets() {
    assert_eq!(list(&[]), "[]");
}

#[test]
fn value_renders_python_literals() {
    assert_eq!(value(&Json::Null), "None");
    assert_eq!(value(&Json::Bool(true)), "True");
    assert_eq!(value(&Json::Bool(false)), "False");
    assert_eq!(value(&Json::Number(Number::from(7_i32))), "7");
    assert_eq!(value(&Json::Text("oauth2".to_owned())), "'oauth2'");
}

#[test]
fn value_renders_a_list_in_order() {
    let items = Json::List(vec![
        Json::Text("a".to_owned()),
        Json::Bool(false),
        Json::Null,
    ]);
    assert_eq!(value(&items), "['a', False, None]");
}

#[test]
fn value_renders_an_object_in_field_order() {
    let mut fields = Object::new();
    fields.insert("first".to_owned(), Json::Number(Number::from(1_i32)));
    fields.insert("second".to_owned(), Json::Text("x".to_owned()));
    assert_eq!(value(&Json::Object(fields)), "{'first': 1, 'second': 'x'}");
}
