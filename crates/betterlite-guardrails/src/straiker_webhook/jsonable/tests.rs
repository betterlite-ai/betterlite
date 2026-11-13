#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde::Serialize;

use super::{jsonable_dict, opaque_dict_list, record_dict};
use crate::straiker_webhook::types::StraikerWebhookUsage;

enum Item {
    Plain(&'static str),
    Model(&'static str),
}

enum Scenario {
    Single(Item),
    List(&'static [Item]),
    Opaque(&'static str),
}

fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn model(source: &str) -> StraikerWebhookUsage {
    serde_json::from_str(source).unwrap()
}

fn single(item: &Item) -> Option<Object> {
    match item {
        Item::Plain(source) => jsonable_dict(&parsed(source)),
        Item::Model(source) => record_dict(&model(source)),
    }
}

fn project(scenario: &Scenario) -> Json {
    match scenario {
        Scenario::Single(item) => as_json(&single(item)),
        Scenario::List(items) => {
            let entries: Vec<Json> = items
                .iter()
                .map(|item| match item {
                    Item::Plain(source) => parsed(source),
                    Item::Model(source) => {
                        record_dict(&model(source)).map_or_else(|| Json::Null, Json::Object)
                    }
                })
                .collect();
            as_json(&opaque_dict_list(&Json::List(entries)))
        }
        Scenario::Opaque(source) => as_json(&opaque_dict_list(&parsed(source))),
    }
}

#[rstest]
#[case::plain_object_passes_through(
    Scenario::Single(Item::Plain(r#"{"a":1}"#)),
    parsed(r#"{"a":1}"#)
)]
#[case::plain_non_object_is_absent(Scenario::Single(Item::Plain(r#""text""#)), parsed("null"))]
#[case::model_record_drops_unset_fields(
    Scenario::Single(Item::Model(r#"{"input_tokens":3}"#)),
    parsed(r#"{"input_tokens":3}"#)
)]
#[case::empty_model_record_is_an_empty_object(Scenario::Single(Item::Model("{}")), parsed("{}"))]
#[case::list_keeps_objects_and_models(
    Scenario::List(&[
        Item::Plain(r#"{"role":"user"}"#),
        Item::Model(r#"{"input_tokens":4,"output_tokens":2}"#),
        Item::Plain(r#""text""#),
        Item::Plain("null"),
        Item::Plain("[1,2]"),
    ]),
    parsed(r#"[{"role":"user"},{"input_tokens":4,"output_tokens":2}]"#)
)]
#[case::list_without_objects_is_absent(
    Scenario::List(&[Item::Plain(r#""text""#), Item::Plain("null"), Item::Plain("5")]),
    parsed("null")
)]
#[case::empty_list_is_absent(Scenario::List(&[]), parsed("null"))]
#[case::opaque_non_list_is_absent(Scenario::Opaque(r#"{"a":1}"#), parsed("null"))]
fn straiker_webhook_context_tests_behavior(#[case] scenario: Scenario, #[case] expected: Json) {
    assert_eq!(project(&scenario), expected);
}
