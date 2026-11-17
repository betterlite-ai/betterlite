use rstest::rstest;
use uuid::Uuid;

use super::*;
#[test]
fn cohere_v2_chat_strips_the_version_prefix_and_frames_the_request() {
    let prepared = ChatFixture::new("v2/v2/command-r").prepared();
    assert_eq!(url_of(&prepared), "https://api.cohere.com/v2/chat");
    let body = body_of(&prepared);
    assert_eq!(text_field(&body, "model").as_deref(), Some("command-r"));
    let messages = items_field(&body, "messages");
    let first = messages[0].as_object().unwrap();
    assert_eq!(text_field(first, "content").as_deref(), Some("hi"));
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer key")
    );
    assert_eq!(
        header_of(&prepared, "request-source").as_deref(),
        Some("unspecified:litellm")
    );
    assert_eq!(
        header_of(&prepared, "accept").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        header_of(&prepared, "content-type").as_deref(),
        Some("application/json")
    );
}

#[test]
fn cohere_v1_chat_keeps_its_model_and_sends_a_message() {
    let prepared = ChatFixture::new("v1/command-r").prepared();
    assert_eq!(url_of(&prepared), "https://api.cohere.ai/v1/chat");
    let body = body_of(&prepared);
    assert_eq!(text_field(&body, "message").as_deref(), Some("hi"));
    assert!(body.get("model").is_none());
    assert!(body.get("messages").is_none());
}

#[test]
fn the_chat_base_falls_through_the_globals_and_then_the_route_default() {
    let mut fixture = ChatFixture::new("command-r");
    fixture.setup.settings.api_base = Some(String::from("https://litellm/v1/chat"));
    fixture.setup.settings.cohere_api_base = Some(String::from("https://cohere/v1/chat"));
    assert_eq!(url_of(&fixture.prepared()), "https://litellm/v1/chat");
    fixture.setup.settings.api_base = None;
    assert_eq!(url_of(&fixture.prepared()), "https://cohere/v1/chat");
    fixture.setup.settings.cohere_api_base = None;
    assert_eq!(
        url_of(&fixture.prepared()),
        "https://api.cohere.com/v2/chat"
    );
    fixture.setup.api_base = Some(String::from("https://explicit/v1/chat"));
    fixture.setup.settings.api_base = Some(String::from("https://litellm/v1/chat"));
    fixture.setup.settings.cohere_api_base = Some(String::from("https://cohere/v1/chat"));
    assert_eq!(url_of(&fixture.prepared()), "https://explicit/v1/chat");
}

#[test]
fn the_chat_key_falls_through_the_globals_in_order() {
    let mut fixture = ChatFixture::new("command-r");
    fixture.setup.api_key = Some(key(""));
    fixture.setup.settings.cohere_key = Some(key("first"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer first")
    );
    fixture.setup.settings.cohere_key = Some(key(""));
    fixture.setup.settings.cohere_api_key = Some(key("second"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer second")
    );
    fixture.setup.settings.cohere_api_key = Some(key(""));
    fixture.setup.settings.co_api_key = Some(key("third"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer third")
    );
    fixture.setup.settings.co_api_key = Some(key(""));
    fixture.setup.settings.api_key = Some(key("last"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer last")
    );
    fixture.setup.settings.api_key = Some(key(""));
    assert!(header_of(&fixture.prepared(), "authorization").is_none());
    fixture.setup.api_key = None;
    fixture.setup.settings.api_key = None;
    assert!(header_of(&fixture.prepared(), "authorization").is_none());
    fixture.setup.api_key = Some(key("explicit"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer explicit")
    );
}

#[test]
fn caller_headers_win_over_the_globals_and_extra_headers_overlay() {
    let mut fixture = ChatFixture::new("command-r");
    fixture.setup.settings.headers = headers(&[("x-global", "global"), ("x-shared", "global")]);
    fixture.setup.headers = Some(headers(&[("x-call", "call"), ("x-shared", "call")]));
    fixture.setup.extra = headers(&[("x-extra", "extra"), ("x-shared", "extra")]);
    let prepared = fixture.prepared();
    assert_eq!(header_of(&prepared, "x-call").as_deref(), Some("call"));
    assert_eq!(header_of(&prepared, "x-extra").as_deref(), Some("extra"));
    assert_eq!(header_of(&prepared, "x-shared").as_deref(), Some("extra"));
    assert!(header_of(&prepared, "x-global").is_none());
}

#[test]
fn an_empty_caller_headers_map_falls_back_to_the_globals_and_extra_headers() {
    let mut fixture = ChatFixture::new("command-r");
    fixture.setup.settings.headers = headers(&[("x-global", "global")]);
    fixture.setup.headers = Some(Headers::new());
    fixture.setup.extra = headers(&[("x-extra", "extra")]);
    let prepared = fixture.prepared();
    assert_eq!(header_of(&prepared, "x-global").as_deref(), Some("global"));
    assert_eq!(header_of(&prepared, "x-extra").as_deref(), Some("extra"));
}

#[test]
fn extra_headers_are_merged_over_the_globals_when_no_caller_headers_arrive() {
    let mut fixture = ChatFixture::new("command-r");
    fixture.setup.settings.headers = headers(&[("x-global", "global"), ("x-shared", "global")]);
    fixture.setup.extra = headers(&[("x-extra", "extra"), ("x-shared", "extra")]);
    let prepared = fixture.prepared();
    assert_eq!(header_of(&prepared, "x-extra").as_deref(), Some("extra"));
    assert_eq!(header_of(&prepared, "x-global").as_deref(), Some("global"));
    assert_eq!(header_of(&prepared, "x-shared").as_deref(), Some("extra"));
}

#[test]
fn cohere_chat_overwrites_the_framing_headers() {
    let mut fixture = ChatFixture::new("command-r");
    fixture.setup.headers.replace(headers(&[
        ("Request-Source", "caller"),
        ("accept", "text/plain"),
    ]));
    let prepared = fixture.prepared();
    assert_eq!(
        header_of(&prepared, "request-source").as_deref(),
        Some("unspecified:litellm")
    );
    assert_eq!(
        header_of(&prepared, "accept").as_deref(),
        Some("application/json")
    );
}

#[test]
fn a_stream_request_sends_the_stream_flag() {
    let mut fixture = ChatFixture::new("command-r");
    fixture.stream = true;
    assert_eq!(
        flag_field(&body_of(&fixture.prepared()), "stream"),
        Some(true)
    );
    // the route's request kind decides streaming, so a request field alone sends no flag
    let mut buffered = ChatFixture::new("command-r");
    buffered.request =
        ChatRequest::parse(br#"{"messages":[{"role":"user","content":"hi"}],"stream":true}"#)
            .unwrap();
    assert!(body_of(&buffered.prepared()).get("stream").is_none());
    assert!(
        body_of(&ChatFixture::new("command-r").prepared())
            .get("stream")
            .is_none()
    );
}

#[test]
fn null_passthrough_fields_never_reach_the_body() {
    let mut fixture = ChatFixture::new("command-r");
    fixture.request =
        ChatRequest::parse(br#"{"messages":[{"role":"user","content":"hi"}],"custom":null}"#)
            .unwrap();
    assert!(body_of(&fixture.prepared()).get("custom").is_none());
    // an unset typed field serializes as null, so the mapping must drop it before the family sees it
    assert!(
        body_of(&ChatFixture::new("command-r").prepared())
            .get("temperature")
            .is_none()
    );
}

#[rstest]
#[case::v2("command-r")]
#[case::v1("v1/command-r")]
fn a_parameter_the_caller_set_reaches_the_body(#[case] model: &str) {
    let mut fixture = ChatFixture::new(model);
    fixture.request = ChatRequest::parse(
        br#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":16,"n":2}"#,
    )
    .unwrap();
    let body = body_of(&fixture.prepared());
    assert_eq!(number_field(&body, "max_tokens"), Some(16), "{model}");
    assert_eq!(number_field(&body, "num_generations"), Some(2), "{model}");
}

#[test]
fn a_non_cohere_provider_prepares_no_chat() {
    let mut fixture = ChatFixture::new("voyage-3");
    fixture.setup.provider = ProviderId::Voyage;
    assert!(matches!(
        fixture.prepare(),
        Err(FamilyError::Unsupported { .. })
    ));
}
#[test]
fn a_cohere_v1_reply_is_decoded() {
    let prepared = ChatFixture::new("v1/command-r").prepared();
    let answer = embedding_result(decode(&prepared, 200, V1_CHAT_REPLY).unwrap());
    let EmbeddingRerankResult::CohereV1Chat(reply) = answer else {
        panic!("the v1 chat reply");
    };
    assert_eq!(reply.content.as_deref(), Some("hello"));
    assert_eq!(reply.model, "v1/command-r");
    assert!(reply.id.starts_with("chatcmpl-"));
}

#[test]
fn a_cohere_v2_reply_is_decoded() {
    let prepared = ChatFixture::new("v2/command-r").prepared();
    let answer = embedding_result(decode(&prepared, 200, V2_CHAT_REPLY).unwrap());
    let EmbeddingRerankResult::CohereV2Chat(reply) = answer else {
        panic!("the v2 chat reply");
    };
    assert_eq!(reply.message.content.as_deref(), Some("hello"));
    assert_eq!(reply.model, "command-r");
}

// LiteLLM's v2 decoder mints a uuid for a tool call whose declaration left the id out
#[test]
fn a_v2_reply_mints_the_missing_tool_call_id() {
    let prepared = ChatFixture::new("v2/command-r").prepared();
    let reply = r#"{"message":{"tool_calls":[{"id":"call-1","type":"function","function":{"name":"f","arguments":"{}"}},{"type":"function","function":{"name":"g","arguments":"{}"}}]},"usage":{"tokens":{"input_tokens":1,"output_tokens":2}}}"#;
    let answer = embedding_result(decode(&prepared, 200, reply).unwrap());
    let EmbeddingRerankResult::CohereV2Chat(decoded) = answer else {
        panic!("the v2 chat reply");
    };
    let calls = decoded.message.tool_calls.unwrap();
    assert_eq!(calls[0].id, "call-1");
    Uuid::parse_str(&calls[1].id).unwrap();
}
