#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_prompts::response_tool_calls::structured::{
    MappedHandler, RouteCallTypes, StructuredMessageHandler,
};
use rstest::rstest;
use serde::Serialize;

use super::request_structured_messages;

const ROUTE: &str = "/v1/routed";
const CALL_TYPE: u32 = 7;

struct Handler {
    messages: Option<Vec<Message>>,
}

impl StructuredMessageHandler for Handler {
    fn get_structured_messages(&self, _request_kwargs: &Object) -> Option<Vec<Message>> {
        self.messages.clone()
    }
}

struct Routes(&'static [(&'static str, &'static [u32])]);

impl RouteCallTypes<u32> for Routes {
    fn call_types_for_route(&self, route: &str) -> Option<Vec<u32>> {
        self.0
            .iter()
            .find(|(name, _)| *name == route)
            .map(|(_, call_types)| call_types.to_vec())
    }
}

enum Scenario {
    Plain(&'static str),
    Mapped(&'static str),
    MappedWithoutMessages(&'static str),
}

fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn object(source: &str) -> Object {
    match parsed(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object")
        }
    }
}

fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn routed() -> Vec<Message> {
    vec![serde_json::from_str(r#"{"role":"user","content":"routed"}"#).unwrap()]
}

fn project(scenario: &Scenario) -> Json {
    let routes = Routes(&[(ROUTE, &[CALL_TYPE]), ("", &[CALL_TYPE])]);
    match scenario {
        Scenario::Plain(source) => as_json(&request_structured_messages(
            &object(source),
            &Routes(&[]),
            &[],
        )),
        Scenario::Mapped(source) | Scenario::MappedWithoutMessages(source) => {
            let messages = match scenario {
                Scenario::Mapped(_) => Some(routed()),
                Scenario::Plain(_) | Scenario::MappedWithoutMessages(_) => None,
            };
            let handler = Handler { messages };
            let mappings = [MappedHandler {
                call_types: &[CALL_TYPE],
                handler: &handler,
            }];
            as_json(&request_structured_messages(
                &object(source),
                &routes,
                &mappings,
            ))
        }
    }
}

#[rstest]
#[case::messages_pass_through(
    Scenario::Plain(r#"{"messages":[{"role":"user","content":"hi"}]}"#),
    parsed(r#"[{"role":"user","content":"hi"}]"#)
)]
#[case::messages_must_be_a_list(Scenario::Plain(r#"{"messages":"hi"}"#), parsed("null"))]
#[case::messages_absent_without_a_route(Scenario::Plain("{}"), parsed("null"))]
#[case::route_without_translation(
    Scenario::Plain(r#"{"litellm_metadata":{"user_api_key_request_route":"/v1/unmapped"}}"#),
    parsed("null")
)]
#[case::empty_route_never_translates(
    Scenario::Mapped(r#"{"litellm_metadata":{"user_api_key_request_route":""}}"#),
    parsed("null")
)]
#[case::route_with_translation_resolves(
    Scenario::Mapped(r#"{"litellm_metadata":{"user_api_key_request_route":"/v1/routed"}}"#),
    parsed(r#"[{"role":"user","content":"routed"}]"#)
)]
#[case::mapped_handler_without_messages(
    Scenario::MappedWithoutMessages(
        r#"{"litellm_metadata":{"user_api_key_request_route":"/v1/routed"}}"#
    ),
    parsed("null")
)]
#[case::untranslated_route_skips_the_handler(
    Scenario::Mapped(r#"{"litellm_metadata":{"user_api_key_request_route":"/v1/unmapped"}}"#),
    parsed("null")
)]
#[case::messages_skip_the_route_check(
    Scenario::Plain(
        r#"{"messages":[{"role":"user","content":"kept"}],"litellm_metadata":{"user_api_key_request_route":"/v1/unmapped"}}"#
    ),
    parsed(r#"[{"role":"user","content":"kept"}]"#)
)]
fn straiker_webhook_context_tests_behavior(#[case] scenario: Scenario, #[case] expected: Json) {
    assert_eq!(project(&scenario), expected);
}
