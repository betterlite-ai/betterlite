use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::snowflake::tests::support::{MediaSource, messages, parsed};

use super::{RequestError, async_transform_request, transform_request};

fn params(entries: &[(&str, Json)]) -> Object {
    Object::from_iter(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone())),
    )
}

fn keys(body: &Object) -> Vec<&str> {
    body.keys().map(String::as_str).collect()
}

#[tokio::test]
async fn the_openai_body_keeps_litellms_key_order() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "snowflake/mistral-large",
        &turns,
        params(&[
            ("temperature", Json::Number(Number::from(0.5))),
            ("stream", Json::Bool(true)),
            ("max_tokens", Json::Number(Number::from(5))),
            ("extra_body", parsed(r#"{"top_k":3}"#)),
        ]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        keys(&body),
        [
            "model",
            "messages",
            "stream",
            "temperature",
            "top_k",
            "max_completion_tokens"
        ]
    );
    assert_eq!(body["model"], Json::Text("mistral-large".to_owned()));
    assert_eq!(body["stream"], Json::Bool(true));
    assert_eq!(body["max_completion_tokens"], Json::Number(Number::from(5)));
}

#[tokio::test]
async fn the_model_prefix_is_cut_case_sensitively_and_the_name_is_not_lowercased() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "Snowflake/claude-3-5-sonnet",
        &turns,
        Object::new(),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        body["model"],
        Json::Text("Snowflake/claude-3-5-sonnet".to_owned())
    );
    let stripped = transform_request(
        "snowflake/mistral-large",
        &turns,
        Object::new(),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(stripped["model"], Json::Text("mistral-large".to_owned()));
    assert_eq!(
        transform_request(
            "snowflake/claude-3-5-sonnet",
            &turns,
            Object::new(),
            &MediaSource::default(),
        )
        .await
        .unwrap()["model"],
        Json::Text("claude-3-5-sonnet".to_owned())
    );
}

#[tokio::test]
async fn a_falsy_stream_value_sends_false() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    for value in [
        Json::Number(Number::from(0)),
        Json::Text(String::new()),
        Json::Null,
    ] {
        let body = transform_request(
            "mistral-large",
            &turns,
            params(&[("stream", value)]),
            &MediaSource::default(),
        )
        .await
        .unwrap();
        assert_eq!(body["stream"], Json::Bool(false));
    }
}

#[tokio::test]
async fn extra_body_overrides_the_body_keys_before_it() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "mistral-large",
        &turns,
        params(&[(
            "extra_body",
            parsed(r#"{"stream":true,"model":"override","temperature":0.1}"#),
        )]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(body["stream"], Json::Bool(true));
    assert_eq!(body["model"], Json::Text("override".to_owned()));
    assert_eq!(body["temperature"], Json::Number(Number::from(0.1)));
}

#[tokio::test]
async fn max_completion_tokens_wins_over_max_tokens_and_is_written_last() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "mistral-large",
        &turns,
        params(&[
            ("max_tokens", Json::Number(Number::from(5))),
            ("max_completion_tokens", Json::Number(Number::from(7))),
            ("extra_body", parsed(r#"{"max_completion_tokens":9}"#)),
        ]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(body["max_completion_tokens"], Json::Number(Number::from(7)));
    assert!(!body.contains_key("max_tokens"));
}

#[tokio::test]
async fn a_zero_max_completion_tokens_falls_through_to_max_tokens() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "mistral-large",
        &turns,
        params(&[
            ("max_completion_tokens", Json::Number(Number::from(0))),
            ("max_tokens", Json::Number(Number::from(5))),
        ]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(body["max_completion_tokens"], Json::Number(Number::from(5)));
}

#[tokio::test]
async fn a_zero_max_completion_tokens_alone_sends_no_limit() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "mistral-large",
        &turns,
        params(&[("max_completion_tokens", Json::Number(Number::from(0)))]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert!(!body.contains_key("max_completion_tokens"));
    assert!(!body.contains_key("max_tokens"));
}

#[tokio::test]
async fn a_null_max_tokens_sends_no_limit() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "mistral-large",
        &turns,
        params(&[("max_tokens", Json::Null)]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert!(!body.contains_key("max_completion_tokens"));
}

#[tokio::test]
async fn a_max_tokens_inside_extra_body_survives_next_to_the_added_limit() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "mistral-large",
        &turns,
        params(&[
            ("max_tokens", Json::Number(Number::from(5))),
            ("extra_body", parsed(r#"{"max_tokens":11}"#)),
        ]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(body["max_tokens"], Json::Number(Number::from(11)));
    assert_eq!(body["max_completion_tokens"], Json::Number(Number::from(5)));
}

#[tokio::test]
async fn a_non_object_extra_body_is_an_error() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let error = transform_request(
        "mistral-large",
        &turns,
        params(&[("extra_body", Json::Number(Number::from(1)))]),
        &MediaSource::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RequestError::ExtraBody));
}

#[tokio::test]
async fn the_claude_body_gets_the_default_max_tokens() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        Object::new(),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(keys(&body), ["model", "messages", "stream", "max_tokens"]);
    assert_eq!(body["max_tokens"], Json::Number(Number::from(4096)));
    assert_eq!(
        body["messages"],
        parsed(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[tokio::test]
async fn a_truthy_max_completion_tokens_becomes_max_tokens_when_none_is_set() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        params(&[("max_completion_tokens", Json::Number(Number::from(9)))]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(body["max_tokens"], Json::Number(Number::from(9)));
    assert!(!body.contains_key("max_completion_tokens"));
}

#[tokio::test]
async fn a_zero_max_completion_tokens_leaves_the_default_in_place() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        params(&[("max_completion_tokens", Json::Number(Number::from(0)))]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(body["max_tokens"], Json::Number(Number::from(4096)));
    assert!(!body.contains_key("max_completion_tokens"));
}

#[tokio::test]
async fn an_existing_max_tokens_keeps_a_truthy_max_completion_tokens_out() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        params(&[
            ("max_completion_tokens", Json::Number(Number::from(9))),
            ("max_tokens", Json::Number(Number::from(4))),
        ]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(body["max_tokens"], Json::Number(Number::from(4)));
}

#[tokio::test]
async fn a_passthrough_system_wins_over_the_extracted_one() {
    let turns =
        messages(r#"[{"role":"system","content":"from messages"},{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        params(&[("system", Json::Text("from params".to_owned()))]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(body["system"], Json::Text("from params".to_owned()));
    assert_eq!(
        body["messages"],
        parsed(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[tokio::test]
async fn a_null_passthrough_system_falls_back_to_the_extracted_list() {
    let turns =
        messages(r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        params(&[("system", Json::Null)]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        body["system"],
        parsed(r#"[{"type":"text","text":"be nice"}]"#)
    );
}

#[tokio::test]
async fn the_body_and_the_system_get_their_cache_control_normalized() {
    let turns = messages(
        r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral","ttl":"5m"}}]}]"#,
    );
    let system = parsed(
        r#"[{"type":"text","text":"be nice","cache_control":{"type":"ephemeral","ttl":"5m"}}]"#,
    );
    let body = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        params(&[("system", system)]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        body["messages"],
        parsed(
            r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]"#
        )
    );
    assert_eq!(
        body["system"],
        parsed(r#"[{"type":"text","text":"be nice","cache_control":{"type":"ephemeral"}}]"#)
    );
}

#[tokio::test]
async fn tools_and_tool_choice_are_converted_for_the_claude_body() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        params(&[
            (
                "tools",
                parsed(
                    r#"[{"type":"function","function":{"name":"f","parameters":{"$schema":"x","type":"object","properties":{}}}}]"#,
                ),
            ),
            (
                "tool_choice",
                parsed(r#"{"type":"function","function":{"name":"f"}}"#),
            ),
        ]),
        &MediaSource::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        body["tools"],
        parsed(r#"[{"name":"f","input_schema":{"type":"object","properties":{}}}]"#)
    );
    assert_eq!(body["tool_choice"], parsed(r#"{"type":"tool","name":"f"}"#));
}

#[tokio::test]
async fn tools_that_are_not_a_list_are_an_error_naming_the_field() {
    let turns = messages(r#"[{"role":"user","content":"hi"}]"#);
    let error = transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        params(&[("tools", Json::Text("nope".to_owned()))]),
        &MediaSource::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RequestError::List { field: "tools" }));
}

#[tokio::test]
async fn the_async_variant_inlines_remote_media_for_claude_models_only() {
    let source =
        MediaSource::with(&[("https://example.test/cat.png", "data:image/png;base64,AAA")]);
    let turns = messages(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/cat.png"}}]}]"#,
    );
    let body = async_transform_request(
        "snowflake/claude-3-5-sonnet",
        &turns,
        Object::new(),
        &source,
    )
    .await
    .unwrap();
    assert_eq!(source.converted(), ["https://example.test/cat.png"]);
    assert_eq!(
        body["messages"],
        parsed(
            r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAA"}}]}]"#
        )
    );
    source.conversions.lock().unwrap().clear();
    let openai = async_transform_request("snowflake/mistral-large", &turns, Object::new(), &source)
        .await
        .unwrap();
    assert!(source.converted().is_empty());
    assert_eq!(
        openai["messages"],
        parsed(
            r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/cat.png"}}]}]"#
        )
    );
}
