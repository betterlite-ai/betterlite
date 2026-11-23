#![cfg(test)]

use betterlite_prompts::arguments::json::Json;

use super::text_outcome;
use crate::genai::session::Outcome;

#[test]
fn text_outcome_writes_the_response_as_json() {
    let outcome = text_outcome(&Json::List(vec![Json::Bool(true)])).unwrap();
    assert!(matches!(outcome, Outcome::Text(text) if text == "[true]"));
}
