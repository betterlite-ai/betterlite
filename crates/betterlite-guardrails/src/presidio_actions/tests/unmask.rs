#![cfg(test)]

use indexmap::IndexMap;

use super::shared::{request_data, tokens};
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::pii_tokens;
use crate::presidio_actions::tokens::unmask_pii_text;

fn token_map(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(token, original)| ((*token).to_owned(), (*original).to_owned()))
        .collect()
}

#[test]
fn numbered_tokens_are_replaced_with_the_original_text() {
    let map = token_map(&[("<PERSON_1>", "Alice"), ("<EMAIL_ADDRESS_2>", "a@b.c")]);
    let text = "Alice <PERSON_1> wrote to <EMAIL_ADDRESS_2>";
    assert_eq!(unmask_pii_text(text, &map), "Alice Alice wrote to a@b.c");
}

#[test]
fn every_occurrence_of_a_token_is_replaced() {
    let map = token_map(&[("<PERSON_1>", "Alice")]);
    assert_eq!(
        unmask_pii_text("<PERSON_1> met <PERSON_1>", &map),
        "Alice met Alice"
    );
}

#[test]
fn map_order_is_kept_so_a_longer_token_goes_first() {
    let map = token_map(&[("<PERSON_10>", "Ten"), ("<PERSON_1>", "One")]);
    assert_eq!(
        unmask_pii_text("A <PERSON_1> B <PERSON_10>", &map),
        "A One B Ten"
    );
}

#[test]
fn an_empty_map_leaves_the_text_alone() {
    assert_eq!(
        unmask_pii_text("nothing here", &IndexMap::new()),
        "nothing here"
    );
}

#[test]
fn tokens_are_read_from_request_metadata_in_order() {
    let data =
        request_data(r#"{"metadata":{"pii_tokens":{"<PERSON_2>":"Bob","<PERSON_1>":"Alice"}}}"#);
    let stored = tokens(&data);
    assert_eq!(
        stored.keys().cloned().collect::<Vec<_>>(),
        vec!["<PERSON_2>".to_owned(), "<PERSON_1>".to_owned()]
    );
    assert_eq!(
        unmask_pii_text("<PERSON_1> and <PERSON_2>", &stored),
        "Alice and Bob"
    );
}

#[test]
fn absent_or_null_tokens_read_as_an_empty_map() {
    assert!(tokens(&request_data("{}")).is_empty());
    assert!(tokens(&request_data(r#"{"metadata":null}"#)).is_empty());
    assert!(tokens(&request_data(r#"{"metadata":{"pii_tokens":null}}"#)).is_empty());
}

#[test]
fn a_pii_tokens_value_that_is_not_an_object_is_an_error() {
    let data = request_data(r#"{"metadata":{"pii_tokens":"text"}}"#);
    let error = pii_tokens(&data).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::PiiTokensNotObject { .. }
    ));
}
