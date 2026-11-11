#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde::Serialize;

use super::{extract_identity, webhook_metadata};

enum Scenario {
    Identity(&'static str),
    Opaque {
        request: &'static str,
        defaults: &'static str,
    },
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

fn project(scenario: &Scenario) -> Json {
    match scenario {
        Scenario::Identity(source) => as_json(&extract_identity(&object(source))),
        Scenario::Opaque { request, defaults } => {
            let configured: BTreeMap<String, String> = serde_json::from_str(defaults).unwrap();
            as_json(&webhook_metadata(&object(request), &configured))
        }
    }
}

#[rstest]
#[case::identity_alias_precedes_hash_and_token(
    Scenario::Identity(
        r#"{"metadata":{"user_api_key_alias":"alias","user_api_key_hash":"hash","user_api_key_token":"token"}}"#
    ),
    parsed(r#"{"litellm_key":"alias"}"#)
)]
#[case::identity_hash_precedes_token(
    Scenario::Identity(
        r#"{"metadata":{"user_api_key_hash":"hash","user_api_key_token":"token"}}"#
    ),
    parsed(r#"{"litellm_key":"hash"}"#)
)]
#[case::identity_team_alias_precedes_team_id(
    Scenario::Identity(
        r#"{"metadata":{"user_api_key_team_alias":"team-alias","user_api_key_team_id":"team-id"}}"#
    ),
    parsed(r#"{"litellm_team":"team-alias"}"#)
)]
#[case::identity_reads_every_field(
    Scenario::Identity(
        r#"{"metadata":{"user_api_key_alias":"alias","user_api_key_team_alias":"team-alias","user_api_key_user_id":"user-7","user_api_key_user_email":"user@example.com","user_api_key_org_id":"org-1","user_api_key_end_user_id":"end-9"}}"#
    ),
    parsed(
        r#"{"litellm_key":"alias","litellm_team":"team-alias","litellm_user_id":"user-7","litellm_user_email":"user@example.com","litellm_org_id":"org-1","end_user_id":"end-9"}"#
    )
)]
#[case::identity_litellm_metadata_overrides_metadata(
    Scenario::Identity(
        r#"{"metadata":{"user_api_key_alias":"old"},"litellm_metadata":{"user_api_key_alias":"new"}}"#
    ),
    parsed(r#"{"litellm_key":"new"}"#)
)]
#[case::identity_ignores_empty_strings(
    Scenario::Identity(r#"{"metadata":{"user_api_key_alias":"","user_api_key_hash":"hash"}}"#),
    parsed(r#"{"litellm_key":"hash"}"#)
)]
#[case::opaque_keeps_scalar_values(
    Scenario::Opaque {
        request: r#"{"metadata":{"name":"plain","count":7,"ratio":0.25,"enabled":false,"absent":null,"nested":{"a":1},"items":[1,2]}}"#,
        defaults: "{}",
    },
    parsed(r#"{"name":"plain","count":7,"ratio":0.25,"enabled":false}"#)
)]
#[case::opaque_filters_application_and_session_keys(
    Scenario::Opaque {
        request: r#"{"metadata":{"agent_id":"agent-1","app_name":"console","session_id":"session-1","user_api_key_alias":"alias","region":"eu"}}"#,
        defaults: "{}",
    },
    parsed(r#"{"region":"eu"}"#)
)]
#[case::opaque_configured_defaults_win(
    Scenario::Opaque {
        request: r#"{"metadata":{"region":"eu"}}"#,
        defaults: r#"{"region":"us","tenant":"acme"}"#,
    },
    parsed(r#"{"region":"us","tenant":"acme"}"#)
)]
#[case::opaque_defaults_without_request_metadata(
    Scenario::Opaque {
        request: "{}",
        defaults: r#"{"tenant":"acme"}"#,
    },
    parsed(r#"{"tenant":"acme"}"#)
)]
#[case::opaque_without_values_is_absent(
    Scenario::Opaque {
        request: r#"{"metadata":{}}"#,
        defaults: "{}",
    },
    parsed("null")
)]
#[case::identity_ignores_non_object_metadata(
    Scenario::Identity(r#"{"metadata":5,"litellm_metadata":"text"}"#),
    parsed("{}")
)]
#[case::opaque_ignores_non_object_metadata(
    Scenario::Opaque {
        request: r#"{"metadata":[]}"#,
        defaults: r#"{"tenant":"acme"}"#,
    },
    parsed(r#"{"tenant":"acme"}"#)
)]
fn straiker_webhook_context_tests_behavior(#[case] scenario: Scenario, #[case] expected: Json) {
    assert_eq!(project(&scenario), expected);
}
