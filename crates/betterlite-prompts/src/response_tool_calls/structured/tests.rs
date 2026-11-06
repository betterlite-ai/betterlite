#![cfg(test)]

use super::{MappedHandler, RouteCallTypes, StructuredMessageHandler, resolve_structured_messages};
use crate::arguments::json::{Json, Object};
use crate::message_content::Message;
use rstest::rstest;

const GIVEN: &str = r#"[{"role":"system","content":"given"}]"#;
const IGNORED: &str = r#"[{"role":"user","content":"ignored"}]"#;
const FIRST: &str = r#"[{"role":"user","content":"first"}]"#;
const SECOND: &str = r#"[{"role":"user","content":"second"}]"#;
const NINE: &str = r#"[{"role":"user","content":"nine"}]"#;
const ROUTED: &str = r#"[{"role":"user","content":"routed"}]"#;

#[derive(Clone, Copy)]
enum Metadata {
    Absent,
    Empty,
    Route(&'static str),
}

struct Scenario {
    messages: Option<&'static str>,
    metadata: Metadata,
    handlers: &'static [(&'static [u32], Option<&'static str>)],
    routes: &'static [(&'static str, &'static [u32])],
}

struct FixedHandler {
    messages: Option<Vec<Message>>,
}

impl StructuredMessageHandler for FixedHandler {
    fn get_structured_messages(&self, _request_kwargs: &Object) -> Option<Vec<Message>> {
        self.messages.clone()
    }
}

struct FixedRoutes {
    routes: Vec<(String, Vec<u32>)>,
}

impl RouteCallTypes<u32> for FixedRoutes {
    fn call_types_for_route(&self, route: &str) -> Option<Vec<u32>> {
        self.routes
            .iter()
            .find(|(name, _)| name == route)
            .map(|(_, call_types)| call_types.clone())
    }
}

fn messages(json: &str) -> Vec<Message> {
    serde_json::from_str(json).expect("messages")
}

fn object(json: &str) -> Object {
    match serde_json::from_str(json).expect("json object") {
        Json::Object(object) => object,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object")
        }
    }
}

fn request_kwargs(metadata: Metadata) -> Object {
    match metadata {
        Metadata::Absent => Object::new(),
        Metadata::Empty => object(r#"{"litellm_metadata":{}}"#),
        Metadata::Route(route) => object(&format!(
            r#"{{"litellm_metadata":{{"user_api_key_request_route":"{route}"}}}}"#
        )),
    }
}

fn run_scenario(scenario: &Scenario) -> Option<Vec<Message>> {
    let handlers: Vec<FixedHandler> = scenario
        .handlers
        .iter()
        .map(|(_, json)| FixedHandler {
            messages: json.map(messages),
        })
        .collect();
    let mappings: Vec<MappedHandler<'_, u32>> = scenario
        .handlers
        .iter()
        .zip(&handlers)
        .map(|((call_types, _), handler)| MappedHandler {
            call_types,
            handler,
        })
        .collect();
    let routes = FixedRoutes {
        routes: scenario
            .routes
            .iter()
            .map(|(route, call_types)| ((*route).to_owned(), call_types.to_vec()))
            .collect(),
    };
    resolve_structured_messages(
        scenario.messages.map(messages).as_deref(),
        &request_kwargs(scenario.metadata),
        &mappings,
        &routes,
    )
}

#[rstest]
#[case::messages_are_returned_unchanged(
    Scenario {
        messages: Some(GIVEN),
        metadata: Metadata::Route("/chat/completions"),
        handlers: &[(&[1], Some(IGNORED))],
        routes: &[("/chat/completions", &[1])],
    },
    Some(GIVEN)
)]
#[case::route_selects_the_first_mapped_call_type(
    Scenario {
        messages: None,
        metadata: Metadata::Route("/chat/completions"),
        handlers: &[(&[9], Some(NINE)), (&[1], Some(ROUTED))],
        routes: &[("/chat/completions", &[1, 9])],
    },
    Some(ROUTED)
)]
#[case::unmapped_route_tries_every_handler(
    Scenario {
        messages: None,
        metadata: Metadata::Route("/chat/completions"),
        handlers: &[(&[1], None), (&[9], Some(SECOND))],
        routes: &[("/chat/completions", &[7])],
    },
    Some(SECOND)
)]
#[case::a_handler_without_messages_falls_through(
    Scenario {
        messages: None,
        metadata: Metadata::Absent,
        handlers: &[(&[1], None), (&[9], Some(SECOND))],
        routes: &[],
    },
    Some(SECOND)
)]
#[case::no_handler_with_messages_yields_none(
    Scenario {
        messages: None,
        metadata: Metadata::Absent,
        handlers: &[(&[1], None), (&[9], None)],
        routes: &[],
    },
    None
)]
#[case::metadata_without_a_route_tries_every_handler(
    Scenario {
        messages: None,
        metadata: Metadata::Empty,
        handlers: &[(&[1], Some(FIRST))],
        routes: &[],
    },
    Some(FIRST)
)]
#[case::an_empty_route_tries_every_handler(
    Scenario {
        messages: None,
        metadata: Metadata::Route(""),
        handlers: &[(&[1], Some(FIRST))],
        routes: &[],
    },
    Some(FIRST)
)]
fn response_tool_calls_behavior_tests_behavior(
    #[case] scenario: Scenario,
    #[case] expected: Option<&str>,
) {
    assert_eq!(run_scenario(&scenario), expected.map(messages));
}
