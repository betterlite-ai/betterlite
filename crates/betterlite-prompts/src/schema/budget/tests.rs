#![cfg(test)]

use super::FlattenBudget;
use crate::arguments::json::{Json, Object};
use crate::schema::flatten::flatten_and_drop;
use rstest::rstest;
use std::iter::repeat_with;

fn object(text: &str) -> Object {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::Object(value) => value,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("fixture must be an object")
        }
    }
}

#[rstest]
#[case::expansion_bomb(true)]
#[case::work_budget(false)]
fn tool_schemas_tool_schema_tests_behavior(#[case] grow_output: bool) {
    let branches = (0_usize..120_000_usize)
        .map(|index| {
            if grow_output {
                format!(r#"{{"type":"object","properties":{{"p{index}":{{"type":"string"}}}}}}"#)
            } else {
                "{}".to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let input = format!(r#"{{"anyOf":[{branches}]}}"#);
    let schema = object(&input);
    assert_eq!(flatten_and_drop(&schema), schema);
}

#[test]
fn tool_schemas_tool_schema_tests_work_budget_rejects_the_next_unit() {
    let mut budget = FlattenBudget::default();
    let accepted = repeat_with(|| budget.charge_work().is_some())
        .take(100_001_usize)
        .collect::<Vec<_>>();
    assert!(
        accepted
            .iter()
            .take(100_000)
            .all(|is_accepted| *is_accepted)
    );
    assert_eq!(accepted.last().copied(), Some(false));
}
