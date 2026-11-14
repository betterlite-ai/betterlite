use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use crate::completion_bridge::params::{ParamsError, build_completion_params};
use crate::completion_bridge::tests::support::{
    bridge_call, litellm_params, parsed, selector_params, send_params, send_params_with_metadata,
};
use crate::completion_bridge::wire::Role;

#[test]
fn strips_agent_only_params_and_copies_the_rest() {
    let params = litellm_params(&[
        ("model", AgentJson::String("gpt-4o".to_owned())),
        (
            "custom_llm_provider",
            AgentJson::String("openai".to_owned()),
        ),
        ("is_public", parsed("true")),
        ("agent_name", parsed(r#""agent""#)),
        ("agent_id", parsed(r#""agent-1""#)),
        ("agent_card_params", parsed(r#"{"model":"x"}"#)),
        ("agent_card_path", parsed(r#"/v1/agents/agent-1"#)),
        ("litellm_a2a_user_api_key_hash", parsed(r#""hash""#)),
        ("temperature", parsed("0.2")),
        ("api_key", parsed(r#""sk-1""#)),
    ]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        serde_json::to_value(&built).expect("the completion call encodes"),
        parsed(
            r#"{"model":"openai/gpt-4o","messages":[{"role":"user","content":"hello"}],"stream":false,"temperature":0.2,"api_key":"sk-1"}"#
        )
    );
}

#[test]
fn transforms_the_a2a_message_into_one_openai_message() {
    let mut message = send_params();
    message.message.role = Role("agent".to_owned());
    message.message.parts = vec![
        parsed(r#"{"kind":"text","text":"one"}"#),
        parsed(r#"{"kind":"text","text":"two"}"#),
        parsed(r#"{"kind":"image","text":"ignored"}"#),
    ];
    let params = litellm_params(&[]);
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        serde_json::to_value(&built.messages).expect("the messages encode"),
        parsed(r#"[{"role":"agent","content":"one\ntwo"}]"#)
    );
}

#[rstest]
#[case::default_model(None, None, "agent")]
#[case::provider_only(Some("openai"), None, "openai/agent")]
#[case::provider_and_model(Some("openai"), Some("gpt-4o"), "openai/gpt-4o")]
#[case::already_prefixed(Some("openai"), Some("openai/gpt-4o"), "openai/gpt-4o")]
#[case::multi_segment_prefix(
    Some("bedrock"),
    Some("bedrock/agentcore/arn"),
    "bedrock/agentcore/arn"
)]
#[case::empty_provider(Some(""), Some("gpt-4o"), "gpt-4o")]
#[case::empty_model(Some("openai"), Some(""), "openai/")]
#[case::case_sensitive_prefix(Some("OpenAI"), Some("openai/gpt-4o"), "OpenAI/openai/gpt-4o")]
#[case::another_providers_prefix(Some("openai"), Some("azure/gpt-4o"), "openai/azure/gpt-4o")]
fn prepends_the_provider_to_the_model(
    #[case] provider: Option<&str>,
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    let params = selector_params(provider, model);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        built.model, expected,
        "provider {provider:?} with model {model:?}"
    );
}

#[rstest]
#[case::null_model(r#"{"custom_llm_provider":"openai","model":null}"#, "openai/agent")]
#[case::null_provider(r#"{"custom_llm_provider":null,"model":"gpt-4o"}"#, "gpt-4o")]
fn null_selector_fields_take_the_default(#[case] raw: &str, #[case] expected: &str) {
    let params: BTreeMap<String, AgentJson> =
        serde_json::from_str(raw).expect("the fixture params are JSON");
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(built.model, expected);
}

#[rstest]
#[case::model_number(litellm_params(&[("model", parsed("5"))]), "model")]
#[case::provider_bool(litellm_params(&[("custom_llm_provider", parsed("true"))]), "custom_llm_provider")]
fn rejects_non_text_selector_fields(
    #[case] params: BTreeMap<String, AgentJson>,
    #[case] field: &'static str,
) {
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let failure =
        build_completion_params(&call, false).expect_err("the fixture selector fields are invalid");
    assert_eq!(failure, ParamsError::NotText { field });
}

#[rstest]
#[case::card_api_base(
    None,
    Some("https://card.example/v1"),
    Some(parsed(r#""https://card.example/v1""#))
)]
#[case::configured_api_base_wins(
    Some(parsed(r#""https://configured.example/v1""#)),
    Some("https://card.example/v1"),
    Some(parsed(r#""https://configured.example/v1""#))
)]
#[case::configured_null_suppresses_the_card(
    Some(AgentJson::Null),
    Some("https://card.example/v1"),
    Some(AgentJson::Null)
)]
#[case::no_api_base(None, None, None)]
fn forwards_the_card_api_base_unless_configured(
    #[case] configured: Option<AgentJson>,
    #[case] card: Option<&str>,
    #[case] expected: Option<AgentJson>,
) {
    let mut params = litellm_params(&[("model", AgentJson::String("gpt-4o".to_owned()))]);
    if let Some(api_base) = configured {
        params.insert("api_base".to_owned(), api_base);
    }
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    call.api_base = card;
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(built.extra.get("api_base"), expected.as_ref());
}

#[test]
fn layers_metadata_with_configured_keys_on_top() {
    let params = litellm_params(&[
        ("model", AgentJson::String("gpt-4o".to_owned())),
        ("metadata", parsed(r#"{"spend":true}"#)),
        (
            "extra_body",
            parsed(r#"{"custom":1,"metadata":{"same":"configured","owner":"server"}}"#),
        ),
    ]);
    let message = send_params_with_metadata(
        Some(parsed(
            r#"{"message":true,"same":"message","clash":"message"}"#,
        )),
        Some(parsed(
            r#"{"params":true,"same":"params","clash":"params"}"#,
        )),
    );
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        serde_json::to_value(&built).expect("the completion call encodes"),
        parsed(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}],"stream":false,"metadata":{"spend":true},"extra_body":{"custom":1,"metadata":{"params":true,"message":true,"clash":"message","same":"configured","owner":"server"}}}"#
        )
    );
}

#[test]
fn adds_extra_body_for_forwarded_metadata() {
    let params = litellm_params(&[]);
    let message = send_params_with_metadata(Some(parsed(r#"{"a":1}"#)), None);
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        built.extra.get("extra_body"),
        Some(&parsed(r#"{"metadata":{"a":1}}"#))
    );
}

#[test]
fn keeps_configured_extra_body_without_forwarded_metadata() {
    let params = litellm_params(&[(
        "extra_body",
        parsed(r#"{"metadata":{"only":"configured"},"custom":1}"#),
    )]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        built.extra.get("extra_body"),
        Some(&parsed(r#"{"metadata":{"only":"configured"},"custom":1}"#))
    );
}

#[test]
fn configured_messages_replace_the_transformed_message() {
    let params = litellm_params(&[(
        "messages",
        parsed(r#"[{"role":"system","content":"custom"},{"role":"user","content":"second"}]"#),
    )]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        serde_json::to_value(&built.messages).expect("the messages encode"),
        parsed(r#"[{"role":"system","content":"custom"},{"role":"user","content":"second"}]"#)
    );
}

#[test]
fn null_messages_use_the_transformed_message() {
    let params = litellm_params(&[("messages", AgentJson::Null)]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        serde_json::to_value(&built.messages).expect("the messages encode"),
        parsed(r#"[{"role":"user","content":"hello"}]"#)
    );
}

#[rstest]
#[case::not_a_list(parsed(r#""hello""#))]
#[case::missing_content(parsed(r#"[{"role":"user"}]"#))]
#[case::non_string_role(parsed(r#"[{"role":1,"content":"x"}]"#))]
fn rejects_configured_messages_of_the_wrong_shape(#[case] messages: AgentJson) {
    let params = litellm_params(&[("messages", messages)]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let failure =
        build_completion_params(&call, false).expect_err("the fixture messages are invalid");
    assert_eq!(failure, ParamsError::NotMessages { field: "messages" });
}

#[test]
fn merges_agent_headers_case_insensitively() {
    let params = litellm_params(&[
        ("model", AgentJson::String("gpt-4o".to_owned())),
        (
            "extra_headers",
            parsed(r#"{"authorization":"static","X-Static":"static"}"#),
        ),
    ]);
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    let dynamic = BTreeMap::from([
        ("X-Trace".to_owned(), "dynamic".to_owned()),
        ("Authorization".to_owned(), "dynamic".to_owned()),
        ("x-keep".to_owned(), "keep".to_owned()),
    ]);
    call.agent_extra_headers = Some(&dynamic);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        built.extra.get("extra_headers"),
        Some(&parsed(
            r#"{"X-Static":"static","X-Trace":"dynamic","authorization":"static","x-keep":"keep"}"#
        ))
    );
}

#[test]
fn agent_headers_alone_become_the_extra_headers() {
    let params = litellm_params(&[("model", AgentJson::String("gpt-4o".to_owned()))]);
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    let dynamic = BTreeMap::from([("X-Trace".to_owned(), "dynamic".to_owned())]);
    call.agent_extra_headers = Some(&dynamic);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        built.extra.get("extra_headers"),
        Some(&parsed(r#"{"X-Trace":"dynamic"}"#))
    );
}

#[rstest]
#[case::no_headers(None)]
#[case::empty_headers(Some(BTreeMap::new()))]
fn keeps_configured_headers_when_no_agent_headers_arrive(
    #[case] agent_headers: Option<BTreeMap<String, String>>,
) {
    let params = litellm_params(&[("extra_headers", parsed(r#"{"X-Static":"static"}"#))]);
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    call.agent_extra_headers = agent_headers.as_ref();
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        built.extra.get("extra_headers"),
        Some(&parsed(r#"{"X-Static":"static"}"#))
    );
}

#[test]
fn null_configured_headers_leave_the_dynamic_ones() {
    let params = litellm_params(&[("extra_headers", AgentJson::Null)]);
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    let dynamic = BTreeMap::from([("X-Trace".to_owned(), "dynamic".to_owned())]);
    call.agent_extra_headers = Some(&dynamic);
    let built = build_completion_params(&call, false).expect("the fixture params build");
    assert_eq!(
        built.extra.get("extra_headers"),
        Some(&parsed(r#"{"X-Trace":"dynamic"}"#))
    );
}

#[rstest]
#[case::not_a_map(parsed(r#""bad""#))]
#[case::non_string_value(parsed(r#"{"X-Static":1}"#))]
fn rejects_configured_headers_of_the_wrong_shape(#[case] extra_headers: AgentJson) {
    let params = litellm_params(&[("extra_headers", extra_headers)]);
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    let dynamic = BTreeMap::from([("X-Trace".to_owned(), "dynamic".to_owned())]);
    call.agent_extra_headers = Some(&dynamic);
    let failure =
        build_completion_params(&call, false).expect_err("the fixture headers are invalid");
    assert_eq!(
        failure,
        ParamsError::NotHeaders {
            field: "extra_headers"
        }
    );
}

#[test]
fn the_entry_point_decides_the_stream_mode() {
    let params = litellm_params(&[
        ("model", AgentJson::String("gpt-4o".to_owned())),
        ("stream", parsed("true")),
        ("stream_options", parsed(r#"{"include_usage":true}"#)),
    ]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let streamed = build_completion_params(&call, true).expect("the fixture params build");
    assert!(streamed.stream);
    let plain = build_completion_params(&call, false).expect("the fixture params build");
    assert!(!plain.stream);
    assert!(!plain.extra.contains_key("stream"));
    assert_eq!(
        plain.extra.get("stream_options"),
        Some(&parsed(r#"{"include_usage":true}"#))
    );
}
