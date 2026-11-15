#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde_json::Number;

use super::support::{Capabilities, Query, Warnings, capture_warnings, runtime};
use crate::invoke_schema::{
    apply_structured_output, ensure_tool_names, inline_output_format,
    normalize_schema_custom_types, normalize_tool_custom_field, normalize_tool_schema_types,
    pop_output_config_format, strip_eager_input_streaming, strip_unsupported_config_keys,
};

#[derive(Clone, Copy, Debug)]
enum Scenario {
    InlineOutputSchema,
    OutputConfigUnsupportedKeys,
    CustomToolField,
    EagerStreamingRemoved,
    SchemaCustomTypes,
    MissingToolName,
    NativeOutputFormatSupport,
}

#[rstest]
#[case::inline_output_schema(Scenario::InlineOutputSchema)]
#[case::output_config_unsupported_keys(Scenario::OutputConfigUnsupportedKeys)]
#[case::custom_tool_field(Scenario::CustomToolField)]
#[case::eager_streaming_removed(Scenario::EagerStreamingRemoved)]
#[case::schema_custom_types(Scenario::SchemaCustomTypes)]
#[case::missing_tool_name(Scenario::MissingToolName)]
#[case::native_output_format_support(Scenario::NativeOutputFormatSupport)]
fn invoke_schema_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::InlineOutputSchema => test_inline_output_schema(),
        Scenario::OutputConfigUnsupportedKeys => test_output_config_unsupported_keys(),
        Scenario::CustomToolField => test_custom_tool_field(),
        Scenario::EagerStreamingRemoved => test_eager_streaming_removed(),
        Scenario::SchemaCustomTypes => test_schema_custom_types(),
        Scenario::MissingToolName => test_missing_tool_name(),
        Scenario::NativeOutputFormatSupport => test_native_output_format_support(),
    }
}

fn object(entries: &[(&str, Json)]) -> Object {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn json_object(entries: &[(&str, Json)]) -> Json {
    Json::Object(object(entries))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn list(items: Vec<Json>) -> Json {
    Json::List(items)
}

fn flag(value: bool) -> Json {
    Json::Bool(value)
}

fn type_custom() -> Json {
    json_object(&[("type", text("custom"))])
}

fn type_object() -> Json {
    json_object(&[("type", text("object"))])
}

fn string_schema() -> Json {
    json_object(&[("type", text("string"))])
}

fn schema_format_object(schema: Json) -> Object {
    object(&[("type", text("json_schema")), ("schema", schema)])
}

fn schema_format(schema: Json) -> Json {
    Json::Object(schema_format_object(schema))
}

fn block(value: &str) -> Json {
    json_object(&[("type", text("text")), ("text", text(value))])
}

fn messages_of(body: &Object) -> Vec<Json> {
    match body.get("messages") {
        Some(Json::List(items)) => items.clone(),
        _ => panic!("messages must be a list"),
    }
}

fn content_of(body: &Object, index: usize) -> Vec<Json> {
    match messages_of(body).get(index) {
        Some(Json::Object(fields)) => match fields.get("content") {
            Some(Json::List(items)) => items.clone(),
            _ => panic!("content must be a list"),
        },
        _ => panic!("message must be an object"),
    }
}

fn tools_in(body: &Object) -> Vec<Json> {
    match body.get("tools") {
        Some(Json::List(items)) => items.clone(),
        _ => panic!("tools must be a list"),
    }
}

fn user_message(content: Json) -> Json {
    json_object(&[("role", text("user")), ("content", content)])
}

fn type_format() -> Json {
    schema_format(type_object())
}

fn message_body(messages: Vec<Json>, format: Json) -> Object {
    let output_config = json_object(&[("format", format)]);
    object(&[
        ("messages", list(messages)),
        ("output_config", output_config),
    ])
}

fn tool_with(custom: Json, defer: Option<Json>) -> Json {
    let mut tool = object(&[("name", text("t")), ("custom", custom)]);
    if let Some(value) = defer {
        tool.insert("defer_loading".to_owned(), value);
    }
    Json::Object(tool)
}

fn test_inline_output_schema() {
    let tokio_runtime = runtime();
    let (guard, warnings) = capture_warnings();
    tokio_runtime.block_on(async {
        nested_format_is_inlined().await;
        legacy_dict_wins().await;
        empty_legacy_hides_the_nested_schema().await;
        non_dict_legacy_falls_back_to_the_nested_format().await;
        non_dict_format_is_dropped().await;
        only_dict_formats_are_returned();
        missing_content_becomes_one_block().await;
        invalid_content_leaves_the_message_alone().await;
        no_user_message_leaves_the_body_alone().await;
        empty_or_non_list_messages_are_not_inlined().await;
        empty_schema_is_not_inlined().await;
        converter_finds_the_last_user_message();
    });
    let captured = warnings.messages();
    assert!(
        captured
            .iter()
            .all(|message| message.contains("supports_native_structured_output"))
    );
    assert!(!captured.is_empty());
    drop(guard);
}

async fn nested_format_is_inlined() {
    let mut capabilities = Capabilities::default();
    let mut body = object(&[
        (
            "messages",
            list(vec![
                json_object(&[("role", text("assistant")), ("content", text("earlier"))]),
                user_message(text("hi")),
            ]),
        ),
        (
            "output_config",
            json_object(&[
                ("format", schema_format(type_object())),
                ("effort", text("high")),
            ]),
        ),
    ]);
    apply_structured_output("plain", &mut body, &mut capabilities).await;
    assert_eq!(
        body.get("output_config"),
        Some(&json_object(&[("effort", text("high"))]))
    );
    assert_eq!(
        content_of(&body, 1),
        vec![block("hi"), block(r#"{"type":"object"}"#)]
    );
}

async fn legacy_dict_wins() {
    let mut capabilities = Capabilities::default();
    let mut body = object(&[
        ("messages", list(vec![user_message(text("hi"))])),
        ("output_format", json_object(&[("schema", type_object())])),
        (
            "output_config",
            json_object(&[("format", schema_format(string_schema()))]),
        ),
    ]);
    apply_structured_output("plain", &mut body, &mut capabilities).await;
    assert!(body.get("output_config").is_none());
    assert!(body.get("output_format").is_none());
    assert_eq!(
        content_of(&body, 0),
        vec![block("hi"), block(r#"{"type":"object"}"#)]
    );
}

async fn empty_legacy_hides_the_nested_schema() {
    let mut body = message_body(vec![user_message(text("hi"))], type_format());
    body.insert("output_format".to_owned(), json_object(&[]));
    apply_structured_output("plain", &mut body, &mut Capabilities::default()).await;
    assert!(body.get("output_config").is_none());
    assert!(body.get("output_format").is_none());
    assert_eq!(messages_of(&body), vec![user_message(text("hi"))]);
}

async fn non_dict_legacy_falls_back_to_the_nested_format() {
    let mut capabilities = Capabilities::default();
    let mut body = object(&[
        (
            "messages",
            list(vec![user_message(list(vec![block("first")]))]),
        ),
        ("output_format", text("ignored")),
        (
            "output_config",
            json_object(&[("format", schema_format(type_object()))]),
        ),
    ]);
    apply_structured_output("plain", &mut body, &mut capabilities).await;
    assert_eq!(
        content_of(&body, 0),
        vec![block("first"), block(r#"{"type":"object"}"#)]
    );
}

async fn non_dict_format_is_dropped() {
    let mut body = message_body(vec![user_message(text("hi"))], text("not-a-dict"));
    apply_structured_output("plain", &mut body, &mut Capabilities::default()).await;
    assert!(body.get("output_config").is_none());
    assert_eq!(messages_of(&body), vec![user_message(text("hi"))]);
}

fn only_dict_formats_are_returned() {
    let mut body = object(&[("output_config", json_object(&[("format", flag(true))]))]);
    assert!(pop_output_config_format(&mut body).is_none());
    assert!(body.get("output_config").is_none());
}

async fn missing_content_becomes_one_block() {
    let mut body = message_body(vec![json_object(&[("role", text("user"))])], type_format());
    apply_structured_output("plain", &mut body, &mut Capabilities::default()).await;
    assert_eq!(content_of(&body, 0), vec![block(r#"{"type":"object"}"#)]);
}

async fn invalid_content_leaves_the_message_alone() {
    for content in [Json::Null, flag(true)] {
        let mut body = message_body(vec![user_message(content.clone())], type_format());
        apply_structured_output("plain", &mut body, &mut Capabilities::default()).await;
        assert_eq!(messages_of(&body), vec![user_message(content)]);
    }
}

async fn no_user_message_leaves_the_body_alone() {
    let assistant = json_object(&[("role", text("assistant"))]);
    let mut body = message_body(vec![assistant.clone()], type_format());
    apply_structured_output("plain", &mut body, &mut Capabilities::default()).await;
    assert_eq!(body, object(&[("messages", list(vec![assistant]))]));
}

async fn empty_or_non_list_messages_are_not_inlined() {
    for messages in [list(Vec::new()), text("messages")] {
        let mut body = object(&[
            ("messages", messages.clone()),
            ("output_config", json_object(&[("format", type_format())])),
        ]);
        apply_structured_output("plain", &mut body, &mut Capabilities::default()).await;
        assert!(body.get("output_config").is_none());
        assert_eq!(body.get("messages"), Some(&messages));
    }
}

async fn empty_schema_is_not_inlined() {
    let mut body = message_body(
        vec![user_message(text("hi"))],
        schema_format(json_object(&[])),
    );
    apply_structured_output("plain", &mut body, &mut Capabilities::default()).await;
    assert_eq!(messages_of(&body), vec![user_message(text("hi"))]);
}

fn converter_finds_the_last_user_message() {
    let mut body = object(&[(
        "messages",
        list(vec![
            json_object(&[("role", text("user")), ("content", text("old"))]),
            json_object(&[("role", text("user"))]),
            text("junk"),
        ]),
    )]);
    inline_output_format(&schema_format_object(type_object()), &mut body);
    let messages = messages_of(&body);
    assert_eq!(
        messages.get(1),
        Some(&json_object(&[
            ("role", text("user")),
            ("content", list(vec![block(r#"{"type":"object"}"#)]))
        ]))
    );
    assert_eq!(messages.first(), Some(&user_message(text("old"))));
}

fn test_output_config_unsupported_keys() {
    let tokio_runtime = runtime();
    let (guard, warnings) = capture_warnings();
    tokio_runtime.block_on(async {
        non_config_shapes_pass_through().await;
        supported_config_keeps_every_key().await;
        effort_flag_keeps_every_key().await;
        unsupported_keys_keep_only_a_non_null_format().await;
        null_and_missing_format_are_removed().await;
    });
    let captured = warnings.messages();
    assert_eq!(captured.len(), 4);
    assert!(
        captured
            .iter()
            .all(|message| message.contains("stripping unsupported `output_config` keys"))
    );
    assert!(
        captured
            .iter()
            .all(|message| message.contains("model=plain"))
    );
    drop(guard);
}

async fn non_config_shapes_pass_through() {
    let mut capabilities = Capabilities::default();
    for output_config in [
        text("config"),
        list(Vec::new()),
        json_object(&[]),
        json_object(&[("format", type_object())]),
    ] {
        let mut body = object(&[("output_config", output_config.clone())]);
        strip_unsupported_config_keys("model", &mut body, &mut capabilities).await;
        assert_eq!(body.get("output_config"), Some(&output_config));
    }
    assert!(capabilities.queries.is_empty());
    assert!(capabilities.effort_models.is_empty());
}

async fn strip_keeps_config(capabilities: &mut Capabilities, model: &str) {
    let config = json_object(&[("effort", text("high")), ("format", type_custom())]);
    let mut body = object(&[("output_config", config.clone())]);
    strip_unsupported_config_keys(model, &mut body, capabilities).await;
    assert_eq!(body.get("output_config"), Some(&config));
}

async fn supported_config_keeps_every_key() {
    let mut capabilities = Capabilities::with_flag("supports_output_config");
    strip_keeps_config(&mut capabilities, "config-model").await;
    assert_eq!(
        capabilities.queries,
        vec![Query {
            model: "config-model".to_owned(),
            provider: Some("bedrock".to_owned()),
            key: "supports_output_config".to_owned(),
        }]
    );
    assert!(capabilities.effort_models.is_empty());
}

async fn effort_flag_keeps_every_key() {
    let mut capabilities = Capabilities {
        factory_flags: Vec::new(),
        effort: true,
        queries: Vec::new(),
        effort_models: Vec::new(),
    };
    strip_keeps_config(&mut capabilities, "effort-model").await;
    assert_eq!(capabilities.effort_models, vec!["effort-model".to_owned()]);
}

async fn unsupported_keys_keep_only_a_non_null_format() {
    for (output_config, expected) in [
        (
            json_object(&[("effort", text("high")), ("format", type_custom())]),
            json_object(&[("format", type_custom())]),
        ),
        (
            json_object(&[("format", flag(false)), ("effort", text("low"))]),
            json_object(&[("format", flag(false))]),
        ),
    ] {
        let mut body = object(&[("output_config", output_config)]);
        strip_unsupported_config_keys("plain", &mut body, &mut Capabilities::default()).await;
        assert_eq!(body.get("output_config"), Some(&expected));
    }
}

async fn null_and_missing_format_are_removed() {
    for output_config in [
        json_object(&[("format", Json::Null), ("effort", text("low"))]),
        json_object(&[("effort", text("low"))]),
    ] {
        let mut body = object(&[("output_config", output_config)]);
        strip_unsupported_config_keys("plain", &mut body, &mut Capabilities::default()).await;
        assert!(body.get("output_config").is_none());
    }
}

fn test_custom_tool_field() {
    invalid_custom_field_shapes_pass_through();
    custom_fields_are_normalized();
}

fn invalid_custom_field_shapes_pass_through() {
    let mut without_tools = object(&[("model", text("m"))]);
    normalize_tool_custom_field(&mut without_tools);
    assert_eq!(without_tools, object(&[("model", text("m"))]));
    for tools in [Json::Null, list(Vec::new()), text("tools")] {
        let mut body = object(&[("tools", tools.clone())]);
        normalize_tool_custom_field(&mut body);
        assert_eq!(body.get("tools"), Some(&tools));
    }
}

fn custom_tools() -> Json {
    list(vec![
        tool_with(json_object(&[("defer_loading", flag(true))]), None),
        tool_with(json_object(&[("defer_loading", flag(false))]), None),
        tool_with(text("custom"), None),
        tool_with(json_object(&[("other", flag(true))]), None),
        tool_with(json_object(&[("defer_loading", Json::Null)]), None),
        tool_with(
            json_object(&[("defer_loading", Json::Null)]),
            Some(Json::Null),
        ),
        tool_with(json_object(&[("defer_loading", text("true"))]), None),
        text("junk"),
    ])
}

fn custom_fields_are_normalized() {
    let mut body = object(&[("tools", custom_tools())]);
    normalize_tool_custom_field(&mut body);
    let tools = tools_in(&body);
    assert_eq!(
        tools.first(),
        Some(&json_object(&[
            ("name", text("t")),
            ("defer_loading", flag(true))
        ]))
    );
    assert_eq!(
        tools.get(1),
        Some(&json_object(&[
            ("name", text("t")),
            ("defer_loading", flag(false))
        ]))
    );
    for index in [2, 3, 4, 6] {
        assert_eq!(tools.get(index), Some(&json_object(&[("name", text("t"))])));
    }
    assert_eq!(
        tools.get(5),
        Some(&json_object(&[
            ("name", text("t")),
            ("defer_loading", Json::Null)
        ]))
    );
    assert_eq!(tools.get(7), Some(&text("junk")));
}

fn test_eager_streaming_removed() {
    non_tool_shapes_yield_none();
    non_dict_entry_invalidates_the_list();
    empty_tools_yield_an_empty_list();
    eager_input_streaming_is_removed();
}

fn non_tool_shapes_yield_none() {
    let without_tools = object(&[("model", text("m"))]);
    assert!(strip_eager_input_streaming(&without_tools).is_none());
    for tools in [
        Json::Null,
        text("tools"),
        flag(true),
        json_object(&[("a", flag(true))]),
    ] {
        let body = object(&[("tools", tools)]);
        assert!(strip_eager_input_streaming(&body).is_none());
    }
}

fn non_dict_entry_invalidates_the_list() {
    let body = object(&[(
        "tools",
        list(vec![json_object(&[("name", text("t"))]), text("junk")]),
    )]);
    assert!(strip_eager_input_streaming(&body).is_none());
}

fn empty_tools_yield_an_empty_list() {
    let body = object(&[("tools", list(Vec::new()))]);
    assert_eq!(strip_eager_input_streaming(&body), Some(Vec::new()));
}

fn eager_input_streaming_is_removed() {
    let body = object(&[(
        "tools",
        list(vec![
            json_object(&[("name", text("t")), ("eager_input_streaming", flag(true))]),
            json_object(&[
                ("name", text("u")),
                ("eager_input_streaming", flag(false)),
                ("description", text("d")),
            ]),
            json_object(&[("name", text("v"))]),
        ]),
    )]);
    assert_eq!(
        strip_eager_input_streaming(&body),
        Some(vec![
            object(&[("name", text("t"))]),
            object(&[("name", text("u")), ("description", text("d"))]),
            object(&[("name", text("v"))]),
        ])
    );
    assert_eq!(
        body.get("tools"),
        Some(&list(vec![
            json_object(&[("name", text("t")), ("eager_input_streaming", flag(true)),]),
            json_object(&[
                ("name", text("u")),
                ("eager_input_streaming", flag(false)),
                ("description", text("d")),
            ]),
            json_object(&[("name", text("v"))]),
        ]))
    );
}

fn test_schema_custom_types() {
    every_schema_edge_is_normalized();
    only_the_named_edges_and_string_custom_change();
    wrapper_normalizes_dict_input_schemas();
    non_tool_shapes_pass_through();
}

fn schema_with(kind: &str) -> Json {
    let typed = json_object(&[("type", text(kind))]);
    json_object(&[
        ("type", text(kind)),
        ("items", typed.clone()),
        ("additionalProperties", typed.clone()),
        (
            "properties",
            json_object(&[("a", typed.clone()), ("b", text("skip"))]),
        ),
        (
            "allOf",
            list(vec![typed.clone(), text("skip"), flag(false)]),
        ),
        ("anyOf", list(vec![typed.clone()])),
        (
            "oneOf",
            list(vec![json_object(&[("type", text(kind)), ("items", typed)])]),
        ),
    ])
}

fn every_schema_edge_is_normalized() {
    let mut schema = schema_with("custom");
    normalize_schema_custom_types(&mut schema);
    assert_eq!(schema, schema_with("object"));
}

fn only_the_named_edges_and_string_custom_change() {
    let mut schema = json_object(&[
        ("type", list(vec![text("custom"), text("null")])),
        ("$defs", json_object(&[("d", type_custom())])),
        ("definitions", json_object(&[("d", type_custom())])),
        ("patternProperties", json_object(&[("d", type_custom())])),
        ("not", type_custom()),
        ("items", list(vec![type_custom()])),
        ("unknown", type_custom()),
        ("properties", text("not-a-map")),
        ("allOf", text("not-a-list")),
    ]);
    let untouched = schema.clone();
    normalize_schema_custom_types(&mut schema);
    assert_eq!(schema, untouched);
}

fn wrapper_normalizes_dict_input_schemas() {
    let mut body = object(&[(
        "tools",
        list(vec![
            json_object(&[("name", text("t")), ("input_schema", type_custom())]),
            json_object(&[("name", text("u"))]),
            json_object(&[("name", text("v")), ("input_schema", text("skip"))]),
            text("junk"),
        ]),
    )]);
    normalize_tool_schema_types(&mut body);
    let tools = tools_in(&body);
    assert_eq!(
        tools.first(),
        Some(&json_object(&[
            ("name", text("t")),
            ("input_schema", type_object())
        ]))
    );
    assert_eq!(tools.get(1), Some(&json_object(&[("name", text("u"))])));
    assert_eq!(
        tools.get(2),
        Some(&json_object(&[
            ("name", text("v")),
            ("input_schema", text("skip"))
        ]))
    );
    assert_eq!(tools.get(3), Some(&text("junk")));
}

fn non_tool_shapes_pass_through() {
    for tools in [Json::Null, text("tools"), list(Vec::new())] {
        let mut body = object(&[("tools", tools.clone())]);
        normalize_tool_schema_types(&mut body);
        assert_eq!(body.get("tools"), Some(&tools));
    }
}

fn test_missing_tool_name() {
    invalid_name_shapes_pass_through();
    blank_names_get_an_indexed_name();
    non_blank_names_are_retained();
}

fn invalid_name_shapes_pass_through() {
    for tools in [Json::Null, text("tools"), json_object(&[("a", flag(true))])] {
        let mut body = object(&[("tools", tools.clone())]);
        ensure_tool_names(&mut body);
        assert_eq!(body.get("tools"), Some(&tools));
    }
}

fn named_tool_body() -> Object {
    let tools = list(vec![
        json_object(&[]),
        json_object(&[("name", Json::Null)]),
        json_object(&[("name", text(""))]),
        json_object(&[("name", text("   "))]),
        json_object(&[("name", text("\u{1c}\u{2003}"))]),
        json_object(&[("name", text(" name "))]),
        json_object(&[("name", Json::Number(Number::from(0_i32)))]),
        json_object(&[("name", flag(false))]),
        text("junk"),
        json_object(&[("input_schema", type_custom())]),
    ]);
    let mut body = object(&[("tools", tools)]);
    normalize_tool_schema_types(&mut body);
    ensure_tool_names(&mut body);
    body
}

fn blank_names_get_an_indexed_name() {
    let tools = tools_in(&named_tool_body());
    for (index, expected) in [
        (0, "litellm_unnamed_tool_0"),
        (1, "litellm_unnamed_tool_1"),
        (2, "litellm_unnamed_tool_2"),
        (3, "litellm_unnamed_tool_3"),
        (4, "litellm_unnamed_tool_4"),
    ] {
        assert_eq!(
            tools.get(index),
            Some(&json_object(&[("name", text(expected))]))
        );
    }
}

fn non_blank_names_are_retained() {
    let tools = tools_in(&named_tool_body());
    assert_eq!(
        tools.get(5),
        Some(&json_object(&[("name", text(" name "))]))
    );
    assert_eq!(
        tools.get(6),
        Some(&json_object(&[("name", Json::Number(Number::from(0_i32)))]))
    );
    assert_eq!(tools.get(7), Some(&json_object(&[("name", flag(false))])));
    assert_eq!(tools.get(8), Some(&text("junk")));
    assert_eq!(
        tools.get(9),
        Some(&json_object(&[
            ("input_schema", type_object()),
            ("name", text("litellm_unnamed_tool_9")),
        ]))
    );
}

fn assert_native_queries(capabilities: &Capabilities) {
    assert!(
        capabilities
            .queries
            .iter()
            .all(|query| query.provider.as_deref() == Some("bedrock"))
    );
    assert!(
        capabilities
            .queries
            .iter()
            .all(|query| query.key == "supports_native_structured_output")
    );
}

fn assert_plain_warnings(warnings: &Warnings) {
    let captured = warnings.messages();
    assert_eq!(captured.len(), 3);
    assert!(
        captured
            .iter()
            .all(|message| message.contains("model=plain"))
    );
    assert!(
        captured
            .iter()
            .all(|message| message
                .contains("does not advertise `supports_native_structured_output`"))
    );
    assert!(
        captured
            .iter()
            .all(|message| message.contains("NOT enforced by the model"))
    );
}

fn test_native_output_format_support() {
    let tokio_runtime = runtime();
    let (guard, warnings) = capture_warnings();
    let mut native = Capabilities::with_flag("supports_native_structured_output");
    tokio_runtime.block_on(async {
        native_restores_the_selected_format(&mut native).await;
        non_dict_or_missing_config_is_replaced(&mut native).await;
    });
    assert_native_queries(&native);
    assert!(warnings.messages().is_empty());
    let mut plain = Capabilities::default();
    tokio_runtime.block_on(async {
        plain_inlines_and_warns(&mut plain).await;
        plain_warns_even_when_nothing_is_appended(&mut plain).await;
        no_dict_format_asks_nothing(&mut plain).await;
    });
    assert_plain_warnings(&warnings);
    drop(guard);
}

async fn native_restores_the_selected_format(capabilities: &mut Capabilities) {
    let config = json_object(&[("effort", text("high")), ("format", type_format())]);
    let mut sibling_body = object(&[
        ("messages", list(vec![user_message(text("hi"))])),
        ("output_config", config.clone()),
    ]);
    apply_structured_output("native-model", &mut sibling_body, capabilities).await;
    assert_eq!(sibling_body.get("output_config"), Some(&config));
    assert_eq!(messages_of(&sibling_body), vec![user_message(text("hi"))]);

    let legacy = json_object(&[("schema", type_object())]);
    let mut legacy_body = object(&[
        ("messages", list(vec![user_message(text("hi"))])),
        ("output_format", legacy.clone()),
        (
            "output_config",
            json_object(&[("format", schema_format(string_schema()))]),
        ),
    ]);
    apply_structured_output("native-model", &mut legacy_body, capabilities).await;
    assert_eq!(
        legacy_body.get("output_config"),
        Some(&json_object(&[("format", legacy)]))
    );
}

async fn non_dict_or_missing_config_is_replaced(capabilities: &mut Capabilities) {
    for config in [Some(text("config")), None] {
        let mut entries = vec![
            ("messages", list(vec![user_message(text("hi"))])),
            ("output_format", schema_format(type_object())),
        ];
        if let Some(value) = config {
            entries.push(("output_config", value));
        }
        let mut body = object(&entries);
        apply_structured_output("native-model", &mut body, capabilities).await;
        assert_eq!(
            body.get("output_config"),
            Some(&json_object(&[("format", schema_format(type_object()))]))
        );
    }
}

async fn plain_inlines_and_warns(capabilities: &mut Capabilities) {
    let mut body = message_body(vec![user_message(text("hi"))], type_format());
    apply_structured_output("plain", &mut body, capabilities).await;
    assert_eq!(
        content_of(&body, 0),
        vec![block("hi"), block(r#"{"type":"object"}"#)]
    );
}

async fn plain_warns_even_when_nothing_is_appended(capabilities: &mut Capabilities) {
    let mut without_messages =
        object(&[("output_config", json_object(&[("format", type_format())]))]);
    apply_structured_output("plain", &mut without_messages, capabilities).await;
    let mut without_user = message_body(
        vec![json_object(&[("role", text("assistant"))])],
        type_format(),
    );
    apply_structured_output("plain", &mut without_user, capabilities).await;
}

async fn no_dict_format_asks_nothing(capabilities: &mut Capabilities) {
    let mut body = object(&[("output_format", text("ignored"))]);
    apply_structured_output("plain", &mut body, capabilities).await;
}
