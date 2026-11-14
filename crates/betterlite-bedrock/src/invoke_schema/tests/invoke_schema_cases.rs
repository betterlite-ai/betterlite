#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::support::{Capabilities, Query, capture_warnings, runtime};
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
    object(&[("type", text("custom"))])
}

fn type_object() -> Json {
    object(&[("type", text("object"))])
}

fn schema_format(schema: Json) -> Json {
    object(&[("type", text("json_schema")), ("schema", schema)])
}

fn block(value: &str) -> Json {
    object(&[("type", text("text")), ("text", text(value))])
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

fn user_message(content: Json) -> Json {
    object(&[("role", text("user")), ("content", content)])
}

fn test_inline_output_schema() {
    let tokio_runtime = runtime();
    let (guard, warnings) = capture_warnings();
    tokio_runtime.block_on(async {
        let mut capabilities = Capabilities::default();

        // the nested format is popped and the schema appended to the last user message
        let mut body = object(&[
            (
                "messages",
                list(vec![
                    object(&[("role", text("assistant")), ("content", text("earlier"))]),
                    user_message(text("hi")),
                ]),
            ),
            (
                "output_config",
                object(&[
                    ("format", schema_format(object(&[("type", text("object"))]))),
                    ("effort", text("high")),
                ]),
            ),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert_eq!(
            body.get("output_config"),
            Some(&object(&[("effort", text("high"))]))
        );
        assert_eq!(
            content_of(&body, 1),
            vec![block("hi"), block(r#"{"type":"object"}"#)]
        );

        // a legacy dict wins over a valid nested format, even when it is empty
        let mut body = object(&[
            ("messages", list(vec![user_message(text("hi"))])),
            ("output_format", object(&[("schema", type_object())])),
            (
                "output_config",
                object(&[("format", schema_format(object(&[("type", text("string"))])))]),
            ),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert!(body.get("output_config").is_none());
        assert!(body.get("output_format").is_none());
        assert_eq!(
            content_of(&body, 0),
            vec![block("hi"), block(r#"{"type":"object"}"#)]
        );

        let mut body = object(&[
            ("messages", list(vec![user_message(text("hi"))])),
            ("output_format", object(&[])),
            (
                "output_config",
                object(&[("format", schema_format(type_object()))]),
            ),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert!(body.get("output_config").is_none());
        assert_eq!(content_of(&body, 0), vec![block("hi")]);

        // a non-dict legacy value lets the nested dict win
        let mut body = object(&[
            (
                "messages",
                list(vec![user_message(list(vec![block("first")]))]),
            ),
            ("output_format", text("ignored")),
            (
                "output_config",
                object(&[("format", schema_format(type_object()))]),
            ),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert_eq!(
            content_of(&body, 0),
            vec![block("first"), block(r#"{"type":"object"}"#)]
        );

        // a non-dict format is popped and leaves nothing behind
        let mut body = object(&[
            ("messages", list(vec![user_message(text("hi"))])),
            ("output_config", object(&[("format", text("not-a-dict"))])),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert!(body.get("output_config").is_none());
        assert_eq!(content_of(&body, 0), vec![block("hi")]);

        // only dict formats are returned
        let mut body = object(&[("output_config", object(&[("format", flag(true))]))]);
        assert!(pop_output_config_format(&mut body).is_none());
        assert!(body.get("output_config").is_none());

        // a missing content becomes a single block, an invalid one leaves the message alone
        let mut body = object(&[
            ("messages", list(vec![object(&[("role", text("user"))])])),
            (
                "output_config",
                object(&[("format", schema_format(type_object()))]),
            ),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert_eq!(content_of(&body, 0), vec![block(r#"{"type":"object"}"#)]);

        for content in [Json::Null, flag(true)] {
            let mut body = object(&[
                ("messages", list(vec![user_message(content)])),
                (
                    "output_config",
                    object(&[("format", schema_format(type_object()))]),
                ),
            ]);
            apply_structured_output("plain", &mut body, &mut capabilities).await;
            assert_eq!(messages_of(&body).len(), 1);
        }

        // no user message, no messages, or a non-list leaves the body untouched
        let mut body = object(&[
            (
                "messages",
                list(vec![object(&[("role", text("assistant"))])]),
            ),
            (
                "output_config",
                object(&[("format", schema_format(type_object()))]),
            ),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert_eq!(
            body,
            object(&[(
                "messages",
                list(vec![object(&[("role", text("assistant"))])])
            )])
        );
        for messages in [list(Vec::new()), text("messages")] {
            let mut body = object(&[
                ("messages", messages),
                (
                    "output_config",
                    object(&[("format", schema_format(type_object()))]),
                ),
            ]);
            apply_structured_output("plain", &mut body, &mut capabilities).await;
            assert!(body.get("output_config").is_none());
        }

        // an empty schema is not inlined, but the nested config is still popped
        let mut body = object(&[
            ("messages", list(vec![user_message(text("hi"))])),
            (
                "output_config",
                object(&[("format", schema_format(object(&[])))]),
            ),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert_eq!(content_of(&body, 0), vec![block("hi")]);

        // the converter alone defaults a missing content and finds the last dict user message
        let mut body = object(&[(
            "messages",
            list(vec![
                object(&[("role", text("user")), ("content", text("old"))]),
                object(&[("role", text("user"))]),
                text("junk"),
            ]),
        )]);
        inline_output_format(&schema_format(type_object()), &mut body);
        let messages = messages_of(&body);
        assert_eq!(
            messages.get(1),
            Some(&object(&[
                ("role", text("user")),
                ("content", list(vec![block(r#"{"type":"object"}"#)]))
            ]))
        );
        assert_eq!(messages.get(0), Some(&user_message(text("old"))));
    });
    let messages = warnings.messages();
    assert!(
        messages
            .iter()
            .all(|message| message.contains("supports_native_structured_output"))
    );
    assert!(!messages.is_empty());
    drop(guard);
}

fn test_output_config_unsupported_keys() {
    let tokio_runtime = runtime();
    let (guard, warnings) = capture_warnings();
    tokio_runtime.block_on(async {
        let mut capabilities = Capabilities::default();
        // shapes that are not a config with extra keys pass through without a lookup
        for output_config in [
            text("config"),
            list(Vec::new()),
            object(&[]),
            object(&[("format", type_object())]),
        ] {
            let mut body = object(&[("output_config", output_config.clone())]);
            strip_unsupported_config_keys("model", &mut body, &mut capabilities).await;
            assert_eq!(body.get("output_config"), Some(&output_config));
        }
        assert!(capabilities.queries.is_empty());
        assert!(capabilities.effort_models.is_empty());

        // supports_output_config keeps every key
        let mut capabilities = Capabilities {
            native: false,
            output_config: true,
            effort: false,
            queries: Vec::new(),
            effort_models: Vec::new(),
        };
        let config = object(&[("effort", text("high")), ("format", type_custom())]);
        let mut body = object(&[("output_config", config.clone())]);
        strip_unsupported_config_keys("config-model", &mut body, &mut capabilities).await;
        assert_eq!(body.get("output_config"), Some(&config));
        assert_eq!(
            capabilities.queries,
            vec![Query {
                model: "config-model".to_owned(),
                provider: Some("bedrock".to_owned()),
                key: "supports_output_config".to_owned(),
            }]
        );
        assert!(capabilities.effort_models.is_empty());

        // an effort tier flag keeps every key after supports_output_config answers false
        let mut capabilities = Capabilities {
            native: false,
            output_config: false,
            effort: true,
            queries: Vec::new(),
            effort_models: Vec::new(),
        };
        let mut body = object(&[("output_config", config.clone())]);
        strip_unsupported_config_keys("effort-model", &mut body, &mut capabilities).await;
        assert_eq!(body.get("output_config"), Some(&config));
        assert_eq!(capabilities.effort_models, vec!["effort-model".to_owned()]);

        // otherwise the keys are dropped, and format survives only when it is not null
        let mut body = object(&[("output_config", config.clone())]);
        strip_unsupported_config_keys("plain", &mut body, &mut Capabilities::default()).await;
        assert_eq!(
            body.get("output_config"),
            Some(&object(&[("format", type_custom())]))
        );

        let mut body = object(&[(
            "output_config",
            object(&[("format", flag(false)), ("effort", text("low"))]),
        )]);
        strip_unsupported_config_keys("plain", &mut body, &mut Capabilities::default()).await;
        assert_eq!(
            body.get("output_config"),
            Some(&object(&[("format", flag(false))]))
        );

        let mut body = object(&[(
            "output_config",
            object(&[("format", Json::Null), ("effort", text("low"))]),
        )]);
        strip_unsupported_config_keys("plain", &mut body, &mut Capabilities::default()).await;
        assert!(body.get("output_config").is_none());

        let mut body = object(&[("output_config", object(&[("effort", text("low"))]))]);
        strip_unsupported_config_keys("plain", &mut body, &mut Capabilities::default()).await;
        assert!(body.get("output_config").is_none());
    });
    let messages = warnings.messages();
    assert_eq!(messages.len(), 4);
    assert!(
        messages
            .iter()
            .all(|message| message.contains("stripping unsupported `output_config` keys"))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.contains("model=plain"))
    );
    drop(guard);
}

fn test_custom_tool_field() {
    let tokio_runtime = runtime();
    tokio_runtime.block_on(async {
        // absent, null, empty and non-list tools stay untouched
        let mut body = object(&[("model", text("m"))]);
        normalize_tool_custom_field(&mut body);
        assert_eq!(body, object(&[("model", text("m"))]));
        for tools in [Json::Null, list(Vec::new()), text("tools")] {
            let mut body = object(&[("tools", tools.clone())]);
            normalize_tool_custom_field(&mut body);
            assert_eq!(body.get("tools"), Some(&tools));
        }

        let tool_with = |custom: Json, defer: Option<Json>| {
            let mut tool = object(&[("name", text("t")), ("custom", custom)]);
            if let Some(value) = defer {
                tool.insert("defer_loading".to_owned(), value);
            }
            tool
        };
        let mut body = object(&[(
            "tools",
            list(vec![
                tool_with(object(&[("defer_loading", flag(true))]), None),
                tool_with(object(&[("defer_loading", flag(false))]), None),
                tool_with(text("custom"), None),
                tool_with(object(&[("other", flag(true))]), None),
                tool_with(object(&[("defer_loading", Json::Null)]), None),
                tool_with(object(&[("defer_loading", Json::Null)]), Some(Json::Null)),
                tool_with(object(&[("defer_loading", text("true"))]), None),
                text("junk"),
            ]),
        )]);
        normalize_tool_custom_field(&mut body);
        let tools = match body.get("tools") {
            Some(Json::List(items)) => items.clone(),
            _ => panic!("tools must be a list"),
        };
        assert_eq!(
            tools.get(0),
            Some(&object(&[
                ("name", text("t")),
                ("defer_loading", flag(true))
            ]))
        );
        assert_eq!(
            tools.get(1),
            Some(&object(&[
                ("name", text("t")),
                ("defer_loading", flag(false))
            ]))
        );
        for index in [2, 3, 4, 6] {
            assert_eq!(tools.get(index), Some(&object(&[("name", text("t"))])));
        }
        assert_eq!(
            tools.get(5),
            Some(&object(&[
                ("name", text("t")),
                ("defer_loading", Json::Null)
            ]))
        );
        assert_eq!(tools.get(7), Some(&text("junk")));
    });
}

fn test_eager_streaming_removed() {
    let tokio_runtime = runtime();
    tokio_runtime.block_on(async {
        let mut body = object(&[("model", text("m"))]);
        assert!(strip_eager_input_streaming(&body).is_none());
        for tools in [
            Json::Null,
            text("tools"),
            flag(true),
            object(&[("a", flag(true))]),
        ] {
            let body = object(&[("tools", tools)]);
            assert!(strip_eager_input_streaming(&body).is_none());
        }
        // one non-dict entry invalidates the whole list instead of salvaging it
        let body = object(&[(
            "tools",
            list(vec![object(&[("name", text("t"))]), text("junk")]),
        )]);
        assert!(strip_eager_input_streaming(&body).is_none());

        let body = object(&[("tools", list(Vec::new()))]);
        assert_eq!(strip_eager_input_streaming(&body), Some(Vec::new()));

        // every value of eager_input_streaming is dropped, and the input stays as it was
        let body = object(&[(
            "tools",
            list(vec![
                object(&[("name", text("t")), ("eager_input_streaming", flag(true))]),
                object(&[
                    ("name", text("u")),
                    ("eager_input_streaming", flag(false)),
                    ("description", text("d")),
                ]),
                object(&[("name", text("v"))]),
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
                object(&[("name", text("t")), ("eager_input_streaming", flag(true))]),
                object(&[
                    ("name", text("u")),
                    ("eager_input_streaming", flag(false)),
                    ("description", text("d"))
                ]),
                object(&[("name", text("v"))]),
            ]))
        );
    });
}

fn test_schema_custom_types() {
    let tokio_runtime = runtime();
    tokio_runtime.block_on(async {
        let mut schema = object(&[
            ("type", text("custom")),
            ("items", type_custom()),
            ("additionalProperties", type_custom()),
            (
                "properties",
                object(&[("a", type_custom()), ("b", text("skip"))]),
            ),
            (
                "allOf",
                list(vec![type_custom(), text("skip"), flag(false)]),
            ),
            ("anyOf", list(vec![type_custom()])),
            (
                "oneOf",
                list(vec![object(&[
                    ("type", text("custom")),
                    ("items", type_custom()),
                ])]),
            ),
        ]);
        normalize_schema_custom_types(&mut schema);
        assert_eq!(
            schema,
            object(&[
                ("type", text("object")),
                ("items", type_object()),
                ("additionalProperties", type_object()),
                (
                    "properties",
                    object(&[("a", type_object()), ("b", text("skip"))])
                ),
                (
                    "allOf",
                    list(vec![type_object(), text("skip"), flag(false)])
                ),
                ("anyOf", list(vec![type_object()])),
                (
                    "oneOf",
                    list(vec![object(&[
                        ("type", text("object")),
                        ("items", type_object())
                    ])])
                ),
            ])
        );

        // only a string "custom" changes, and only on the named schema edges
        let mut schema = object(&[
            ("type", list(vec![text("custom"), text("null")])),
            ("$defs", object(&[("d", type_custom())])),
            ("definitions", object(&[("d", type_custom())])),
            ("patternProperties", object(&[("d", type_custom())])),
            ("not", type_custom()),
            ("items", list(vec![type_custom()])),
            ("unknown", type_custom()),
            ("properties", text("not-a-map")),
            ("allOf", text("not-a-list")),
        ]);
        let untouched = schema.clone();
        normalize_schema_custom_types(&mut schema);
        assert_eq!(schema, untouched);

        // the wrapper only normalizes a dict input_schema of dict tools
        let mut body = object(&[(
            "tools",
            list(vec![
                object(&[("name", text("t")), ("input_schema", type_custom())]),
                object(&[("name", text("u"))]),
                object(&[("name", text("v")), ("input_schema", text("skip"))]),
                text("junk"),
            ]),
        )]);
        normalize_tool_schema_types(&mut body);
        let tools = match body.get("tools") {
            Some(Json::List(items)) => items.clone(),
            _ => panic!("tools must be a list"),
        };
        assert_eq!(
            tools.get(0),
            Some(&object(&[
                ("name", text("t")),
                ("input_schema", type_object())
            ]))
        );
        assert_eq!(tools.get(1), Some(&object(&[("name", text("u"))])));
        assert_eq!(
            tools.get(2),
            Some(&object(&[
                ("name", text("v")),
                ("input_schema", text("skip"))
            ]))
        );
        assert_eq!(tools.get(3), Some(&text("junk")));

        for tools in [Json::Null, text("tools"), list(Vec::new())] {
            let mut body = object(&[("tools", tools.clone())]);
            normalize_tool_schema_types(&mut body);
            assert_eq!(body.get("tools"), Some(&tools));
        }
    });
}

fn test_missing_tool_name() {
    let tokio_runtime = runtime();
    tokio_runtime.block_on(async {
        for tools in [Json::Null, text("tools"), object(&[("a", flag(true))])] {
            let mut body = object(&[("tools", tools.clone())]);
            ensure_tool_names(&mut body);
            assert_eq!(body.get("tools"), Some(&tools));
        }

        let mut body = object(&[(
            "tools",
            list(vec![
                object(&[]),
                object(&[("name", Json::Null)]),
                object(&[("name", text(""))]),
                object(&[("name", text("   "))]),
                object(&[("name", text("\u{1c}\u{2003}"))]),
                object(&[("name", text(" name "))]),
                object(&[("name", Json::Number(0.into()))]),
                object(&[("name", flag(false))]),
                text("junk"),
                object(&[("input_schema", type_custom())]),
            ]),
        )]);
        // the schema the unnamed tool carries is normalized without inventing an error
        normalize_tool_schema_types(&mut body);
        ensure_tool_names(&mut body);
        let tools = match body.get("tools") {
            Some(Json::List(items)) => items.clone(),
            _ => panic!("tools must be a list"),
        };
        assert_eq!(
            tools.get(0),
            Some(&object(&[("name", text("litellm_unnamed_tool_0"))]))
        );
        assert_eq!(
            tools.get(1),
            Some(&object(&[("name", text("litellm_unnamed_tool_1"))]))
        );
        assert_eq!(
            tools.get(2),
            Some(&object(&[("name", text("litellm_unnamed_tool_2"))]))
        );
        assert_eq!(
            tools.get(3),
            Some(&object(&[("name", text("litellm_unnamed_tool_3"))]))
        );
        assert_eq!(
            tools.get(4),
            Some(&object(&[("name", text("litellm_unnamed_tool_4"))]))
        );
        assert_eq!(tools.get(5), Some(&object(&[("name", text(" name "))])));
        assert_eq!(
            tools.get(6),
            Some(&object(&[("name", Json::Number(0.into()))]))
        );
        assert_eq!(tools.get(7), Some(&object(&[("name", flag(false))])));
        assert_eq!(tools.get(8), Some(&text("junk")));
        assert_eq!(
            tools.get(9),
            Some(&object(&[
                ("input_schema", type_object()),
                ("name", text("litellm_unnamed_tool_9")),
            ]))
        );
    });
}

fn test_native_output_format_support() {
    let tokio_runtime = runtime();
    let (guard, warnings) = capture_warnings();
    tokio_runtime.block_on(async {
        // a native model gets the schema back under output_config.format, keeping its sibling keys
        let mut capabilities = Capabilities {
            native: true,
            output_config: false,
            effort: false,
            queries: Vec::new(),
            effort_models: Vec::new(),
        };
        let mut body = object(&[
            ("messages", list(vec![user_message(text("hi"))])),
            (
                "output_config",
                object(&[
                    ("effort", text("high")),
                    ("format", schema_format(type_object())),
                ]),
            ),
        ]);
        apply_structured_output("native-model", &mut body, &mut capabilities).await;
        assert_eq!(
            body.get("output_config"),
            Some(&object(&[
                ("effort", text("high")),
                ("format", schema_format(type_object())),
            ]))
        );
        assert_eq!(content_of(&body, 0), vec![block("hi")]);

        // a non-dict config is replaced, and a missing one is created
        for config in [Some(text("config")), None] {
            let mut entries = vec![
                ("messages", list(vec![user_message(text("hi"))])),
                ("output_format", schema_format(type_object())),
            ];
            if let Some(value) = config {
                entries.push(("output_config", value));
            }
            let mut body = object(&entries);
            apply_structured_output("native-model", &mut body, &mut capabilities).await;
            assert_eq!(
                body.get("output_config"),
                Some(&object(&[("format", schema_format(type_object()))]))
            );
        }

        // the legacy dict still wins for a native model, forwarded exactly as it arrived
        let mut body = object(&[
            ("messages", list(vec![user_message(text("hi"))])),
            ("output_format", object(&[("schema", type_object())])),
            (
                "output_config",
                object(&[("format", schema_format(object(&[("type", text("string"))])))]),
            ),
        ]);
        apply_structured_output("native-model", &mut body, &mut capabilities).await;
        assert_eq!(
            body.get("output_config"),
            Some(&object(&[("format", object(&[("schema", type_object())]))]))
        );

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
        assert!(warnings.messages().is_empty());

        // a model without the flag inlines and warns, even when nothing can be appended
        let mut capabilities = Capabilities::default();
        let mut body = object(&[
            ("messages", list(vec![user_message(text("hi"))])),
            (
                "output_config",
                object(&[("format", schema_format(object(&[("type", text("object"))])))]),
            ),
        ]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
        assert_eq!(
            content_of(&body, 0),
            vec![block("hi"), block(r#"{"type":"object"}"#)]
        );
        for body_entries in [
            vec![(
                "output_config",
                object(&[("format", schema_format(type_object()))]),
            )],
            vec![
                (
                    "messages",
                    list(vec![object(&[("role", text("assistant"))])]),
                ),
                (
                    "output_config",
                    object(&[("format", schema_format(type_object()))]),
                ),
            ],
        ] {
            let mut body = object(&body_entries);
            apply_structured_output("plain", &mut body, &mut capabilities).await;
        }

        // no dict format means no warning and no capability question
        let mut body = object(&[("output_format", text("ignored"))]);
        apply_structured_output("plain", &mut body, &mut capabilities).await;
    });
    let messages = warnings.messages();
    assert_eq!(messages.len(), 3);
    assert!(
        messages
            .iter()
            .all(|message| message.contains("model=plain"))
    );
    assert!(
        messages
            .iter()
            .all(|message| message
                .contains("does not advertise `supports_native_structured_output`"))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.contains("NOT enforced by the model"))
    );
    drop(guard);
}
