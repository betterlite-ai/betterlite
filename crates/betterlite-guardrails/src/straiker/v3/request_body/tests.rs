#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::provider_body;
use crate::straiker::v3::fixtures::{Routes, Tokens, as_json, object, parsed};

fn relayed(source: &str, texts: &[&str]) -> Json {
    let owned: Vec<String> = texts.iter().map(|text| (*text).to_owned()).collect();
    let body = provider_body(&object(source), &owned, &Routes, &Tokens);
    as_json(&body)
}

#[rstest]
#[case::keeps_only_the_client_body(
    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"temperature":0,"deployment":{"api_key":"sk-secret"},"proxy_server_request":{"headers":{"authorization":"Bearer x"}},"metadata":{"a":1},"litellm_call_id":"c"}"#,
    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"temperature":0}"#
)]
#[case::explicit_falsy_values_are_relayed(
    r#"{"stream":false,"n":0,"stop":[],"user":"","messages":[{"role":"user","content":"hi"}]}"#,
    r#"{"stream":false,"n":0,"stop":[],"user":"","messages":[{"role":"user","content":"hi"}]}"#
)]
#[case::tool_and_mcp_credentials_are_redacted(
    r#"{"messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","headers":{"k":"v"},"function":{"name":"f","parameters":{"properties":{"headers":{}}}}}],"mcp_servers":[{"name":"m","authorization_token":"secret"}]}"#,
    r#"{"messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","headers":"[redacted]","function":{"name":"f","parameters":{"properties":{"headers":{}}}}}],"mcp_servers":[{"name":"m","authorization_token":"[redacted]"}]}"#
)]
#[case::other_fields_are_not_redacted(
    r#"{"messages":[{"role":"user","content":"hi"}],"functions":[{"headers":"h"}],"container":{"authorization":"a"}}"#,
    r#"{"messages":[{"role":"user","content":"hi"}],"functions":[{"headers":"h"}],"container":{"authorization":"a"}}"#
)]
#[case::identity_rides_as_metadata(
    r#"{"messages":[{"role":"user","content":"hi"}],"metadata":{"user_api_key_user_email":"a@x","user_api_key_hash":"h","user_api_key_alias":"key"},"litellm_metadata":{"user_api_key_user_id":"u1","user_api_key_team_id":"t","user_api_key_end_user_id":"e"}}"#,
    r#"{"messages":[{"role":"user","content":"hi"}],"metadata":{"user_api_key_end_user_id":"e","user_api_key_user_email":"a@x","user_api_key_user_id":"u1","user_api_key_alias":"key","user_api_key_team_id":"t"}}"#
)]
#[case::master_key_placeholders_are_not_identity(
    r#"{"messages":[{"role":"user","content":"hi"}],"metadata":{"user_api_key_user_id":"default_user_id","user_api_key_alias":"litellm_proxy_master_key","user_api_key_team_id":""}}"#,
    r#"{"messages":[{"role":"user","content":"hi"}]}"#
)]
#[case::completion_prompt_becomes_user_turns(
    r#"{"model":"davinci","prompt":["a","b"],"max_tokens":5,"litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#,
    r#"{"model":"davinci","max_tokens":5,"messages":[{"role":"user","content":"a"},{"role":"user","content":"b"}]}"#
)]
#[case::completion_token_prompt_is_decoded(
    r#"{"prompt":[1,2],"litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#,
    r#"{"messages":[{"role":"user","content":"t1 t2"}]}"#
)]
#[case::completion_prompt_that_cannot_be_rendered_is_relayed(
    r#"{"prompt":[1,500],"litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#,
    r#"{"prompt":[1,500]}"#
)]
#[case::completion_without_a_prompt(
    r#"{"model":"davinci","litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#,
    r#"{"model":"davinci"}"#
)]
#[case::completion_with_messages_keeps_prompt(
    r#"{"prompt":"p","messages":[{"role":"user","content":"hi"}],"litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#,
    r#"{"prompt":"p","messages":[{"role":"user","content":"hi"}]}"#
)]
#[case::prompt_off_the_completions_route_is_relayed(
    r#"{"prompt":"p","litellm_metadata":{"user_api_key_request_route":"/v1/chat/completions"}}"#,
    r#"{"prompt":"p"}"#
)]
fn relays_provider_body(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(relayed(source, &[]), parsed(expected));
}

#[rstest]
#[case::texts_become_user_turns(r#"{"model":"m"}"#, &["a", "", "b"], r#"{"model":"m","messages":[{"role":"user","content":"a"},{"role":"user","content":"b"}]}"#)]
#[case::empty_messages_are_replaced(r#"{"messages":[]}"#, &["a"], r#"{"messages":[{"role":"user","content":"a"}]}"#)]
#[case::only_empty_texts(r#"{"model":"m"}"#, &["", ""], r#"{"model":"m"}"#)]
#[case::conversation_wins(r#"{"messages":[{"role":"user","content":"hi"}]}"#, &["a"], r#"{"messages":[{"role":"user","content":"hi"}]}"#)]
#[case::prompt_wins(r#"{"prompt":"p"}"#, &["a"], r#"{"prompt":"p"}"#)]
#[case::input_wins(r#"{"input":"i"}"#, &["a"], r#"{"input":"i"}"#)]
#[case::null_prompt_is_a_conversation(r#"{"prompt":null}"#, &["a"], r#"{"prompt":null}"#)]
#[case::completion_prompt_wins(r#"{"prompt":"p","litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#, &["a"], r#"{"messages":[{"role":"user","content":"p"}]}"#)]
fn relays_texts_when_there_is_no_conversation(
    #[case] source: &str,
    #[case] texts: &[&str],
    #[case] expected: &str,
) {
    assert_eq!(relayed(source, texts), parsed(expected));
}

#[test]
fn keeps_client_order_then_messages_then_metadata() {
    let source = r#"{"temperature":1,"model":"m","prompt":"p","max_tokens":2,"metadata":{"user_api_key_alias":"k"},"litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#;
    let body = provider_body(&object(source), &[], &Routes, &Tokens);
    let keys: Vec<&str> = body.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["temperature", "model", "max_tokens", "messages", "metadata"]
    );
}
