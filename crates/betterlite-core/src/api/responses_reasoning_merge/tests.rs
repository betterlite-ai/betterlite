#![cfg(test)]

use super::*;
use rstest::rstest;

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut fields = Map::new();
    for (key, value) in pairs {
        fields.insert(key.to_owned(), value);
    }
    Value::Object(fields)
}

fn str_val(value: &str) -> Value {
    Value::String(value.to_owned())
}

fn arr(items: Vec<Value>) -> Value {
    Value::Array(items)
}

fn merge_dicts(messages: Vec<Value>) -> Vec<Value> {
    let mut records: Vec<ChatRecord> = messages
        .into_iter()
        .map(|value| serde_json::from_value(value).expect("test fixture is a chat record"))
        .collect();
    merge_reasoning(&mut records)
        .into_iter()
        .map(|record| serde_json::to_value(record).expect("chat record serializes"))
        .collect()
}

#[test]
fn reasoning_before_answer() {
    let input = vec![
        obj(vec![
            ("role", str_val("assistant")),
            ("content", Value::Null),
            ("reasoning_content", str_val("a")),
            ("thinking_blocks", arr(vec![str_val("A")])),
        ]),
        obj(vec![
            ("role", str_val("assistant")),
            ("content", Value::Null),
            ("reasoning_content", str_val("b")),
            ("thinking_blocks", arr(vec![str_val("B")])),
        ]),
        obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("answer")),
            ("reasoning_content", str_val("c")),
            ("thinking_blocks", arr(vec![str_val("C")])),
            ("tool_calls", arr(vec![str_val("call-1")])),
            ("extra", str_val("retained")),
        ]),
    ];
    let output = merge_dicts(input);
    assert_eq!(
        output,
        vec![obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("answer")),
            ("reasoning_content", str_val("a\nb\nc")),
            (
                "thinking_blocks",
                arr(vec![str_val("A"), str_val("B"), str_val("C")])
            ),
            ("tool_calls", arr(vec![str_val("call-1")])),
            ("extra", str_val("retained")),
        ])]
    );
}

#[test]
fn reasoning_boundaries() {
    let output = merge_dicts(vec![
        obj(vec![
            ("role", str_val("assistant")),
            ("content", Value::Null),
            ("reasoning_content", str_val("before")),
            ("unrelated", str_val("dropped")),
        ]),
        obj(vec![("role", str_val("user")), ("content", str_val("q"))]),
        obj(vec![
            ("role", str_val("assistant")),
            ("content", Value::Null),
            ("thinking_blocks", arr(vec![str_val("B")])),
        ]),
    ]);
    assert_eq!(
        output,
        vec![
            obj(vec![
                ("role", str_val("assistant")),
                ("content", Value::Null),
                ("reasoning_content", str_val("before")),
            ]),
            obj(vec![("role", str_val("user")), ("content", str_val("q"))]),
            obj(vec![
                ("role", str_val("assistant")),
                ("content", Value::Null),
                ("thinking_blocks", arr(vec![str_val("B")])),
            ]),
        ]
    );
}

#[test]
fn reasoning_boundaries_typed_records() {
    let before = AttributeRecord {
        role: Role::Assistant,
        content: None,
        reasoning_content: Some("a".to_owned()),
        thinking_blocks: None,
        tool_calls: None,
    };
    let user = AttributeRecord {
        role: Role::User,
        content: Some("q".to_owned()),
        reasoning_content: None,
        thinking_blocks: None,
        tool_calls: None,
    };
    let after = AttributeRecord {
        role: Role::Assistant,
        content: None,
        reasoning_content: None,
        thinking_blocks: Some(vec![str_val("B")]),
        tool_calls: None,
    };
    let mut records = vec![before.clone(), user.clone(), after.clone()];
    let output = merge_reasoning(&mut records);
    assert_eq!(output, vec![before, user, after]);
}

#[test]
fn reasoning_merges_into_typed_target() {
    let pending = AttributeRecord {
        role: Role::Assistant,
        content: None,
        reasoning_content: Some("a".to_owned()),
        thinking_blocks: Some(vec![str_val("A")]),
        tool_calls: None,
    };
    let target = AttributeRecord {
        role: Role::Assistant,
        content: Some("answer".to_owned()),
        reasoning_content: Some("c".to_owned()),
        thinking_blocks: Some(vec![str_val("C")]),
        tool_calls: None,
    };
    let mut records = vec![pending, target];
    let output = merge_reasoning(&mut records);
    let expected = AttributeRecord {
        role: Role::Assistant,
        content: Some("answer".to_owned()),
        reasoning_content: Some("a\nc".to_owned()),
        thinking_blocks: Some(vec![str_val("A"), str_val("C")]),
        tool_calls: None,
    };
    assert_eq!(output, vec![expected.clone()]);
    // The target element in the original slice is mutated in place, not left as a pending-free clone.
    assert_eq!(records[1], expected);
}

#[test]
fn merge_reasoning_mutates_original_target_record() {
    let mut records = vec![
        ChatRecord {
            role: Role::Assistant,
            content: Value::Null,
            reasoning_content: str_val("a"),
            thinking_blocks: Value::Null,
            tool_calls: Value::Null,
            extra: Map::new(),
        },
        ChatRecord {
            role: Role::Assistant,
            content: str_val("answer"),
            reasoning_content: str_val("c"),
            thinking_blocks: Value::Null,
            tool_calls: Value::Null,
            extra: Map::new(),
        },
    ];
    let output = merge_reasoning(&mut records);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].reasoning_content, str_val("a\nc"));
    assert_eq!(output, vec![records[1].clone()]);
}

#[test]
fn standalone_builder_with_no_reasoning_is_content_null_only() {
    let built = AttributeRecord::blank(None, None);
    assert_eq!(
        built,
        AttributeRecord {
            role: Role::Assistant,
            content: None,
            reasoning_content: None,
            thinking_blocks: None,
            tool_calls: None,
        }
    );
}

#[test]
fn trailing_assistant() {
    let output = merge_trailing(
        &[obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("a")),
            (
                "tool_calls",
                arr(vec![obj(vec![("id", str_val("call-1"))])]),
            ),
            ("keep", Value::Bool(true)),
        ])],
        &[obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("b")),
            ("new", Value::Bool(true)),
        ])],
        false,
    )
    .unwrap();
    assert_eq!(
        output,
        Some(obj(vec![
            ("role", str_val("assistant")),
            (
                "content",
                arr(vec![
                    obj(vec![("type", str_val("text")), ("text", str_val("a"))]),
                    obj(vec![("type", str_val("text")), ("text", str_val("b"))]),
                ])
            ),
            (
                "tool_calls",
                arr(vec![obj(vec![("id", str_val("call-1"))])])
            ),
            ("keep", Value::Bool(true)),
        ]))
    );
}

#[test]
fn trailing_assistant_hosted_search_without_tool_calls() {
    let output = merge_trailing(
        &[obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("a")),
        ])],
        &[obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("b")),
        ])],
        true,
    )
    .unwrap();
    assert_eq!(
        output,
        Some(obj(vec![
            ("role", str_val("assistant")),
            (
                "content",
                arr(vec![
                    obj(vec![("type", str_val("text")), ("text", str_val("a"))]),
                    obj(vec![("type", str_val("text")), ("text", str_val("b"))]),
                ])
            ),
        ]))
    );
}

#[test]
fn trailing_assistant_with_falsy_prior_content_takes_new_content() {
    let output = merge_trailing(
        &[obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("")),
            (
                "tool_calls",
                arr(vec![obj(vec![("id", str_val("call-1"))])]),
            ),
        ])],
        &[obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("b")),
        ])],
        false,
    )
    .unwrap();
    assert_eq!(
        output,
        Some(obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("b")),
            (
                "tool_calls",
                arr(vec![obj(vec![("id", str_val("call-1"))])])
            ),
        ]))
    );
}

#[test]
fn trailing_assistant_preserves_non_text_block() {
    let image_block = obj(vec![
        ("type", str_val("image_url")),
        (
            "image_url",
            obj(vec![("url", str_val("https://example.test/a.png"))]),
        ),
    ]);
    let output = merge_trailing(
        &[obj(vec![
            ("role", str_val("assistant")),
            ("content", arr(vec![image_block.clone()])),
            (
                "tool_calls",
                arr(vec![obj(vec![("id", str_val("call-1"))])]),
            ),
        ])],
        &[assistant_answer("b")],
        false,
    )
    .unwrap();
    assert_eq!(
        output,
        Some(obj(vec![
            ("role", str_val("assistant")),
            (
                "content",
                arr(vec![
                    image_block,
                    obj(vec![("type", str_val("text")), ("text", str_val("b"))]),
                ])
            ),
            (
                "tool_calls",
                arr(vec![obj(vec![("id", str_val("call-1"))])])
            ),
        ]))
    );
}

fn assistant_with_tool_calls() -> Value {
    obj(vec![
        ("role", str_val("assistant")),
        ("content", str_val("a")),
        (
            "tool_calls",
            arr(vec![obj(vec![("id", str_val("call-1"))])]),
        ),
    ])
}

fn assistant_answer(text: &str) -> Value {
    obj(vec![
        ("role", str_val("assistant")),
        ("content", str_val(text)),
    ])
}

#[rstest]
#[case::no_prior_messages(vec![], vec![], false, Ok(None))]
#[case::zero_new_messages(vec![assistant_with_tool_calls()], vec![], false, Ok(None))]
#[case::two_new_messages(
    vec![assistant_with_tool_calls()],
    vec![assistant_answer("b"), assistant_answer("c")],
    false,
    Ok(None)
)]
#[case::typed_non_dict_last_message(
    vec![arr(vec![str_val("not"), str_val("a"), str_val("dict")])],
    vec![assistant_answer("b")],
    false,
    Ok(None)
)]
#[case::new_message_not_dict_raises(
    vec![assistant_with_tool_calls()],
    vec![arr(vec![str_val("not"), str_val("a"), str_val("dict")])],
    false,
    Err(MergeError::InvalidRecord)
)]
#[case::last_message_not_assistant(
    vec![obj(vec![
        ("role", str_val("user")),
        ("content", str_val("a")),
        ("tool_calls", arr(vec![obj(vec![("id", str_val("call-1"))])])),
    ])],
    vec![assistant_answer("b")],
    false,
    Ok(None)
)]
#[case::new_message_not_assistant(
    vec![assistant_with_tool_calls()],
    vec![obj(vec![("role", str_val("user")), ("content", str_val("b"))])],
    false,
    Ok(None)
)]
#[case::no_prior_tool_calls_without_hosted_search(
    vec![assistant_answer("a")],
    vec![assistant_answer("b")],
    false,
    Ok(None)
)]
#[case::truthy_new_tool_calls(
    vec![assistant_with_tool_calls()],
    vec![obj(vec![
        ("role", str_val("assistant")),
        ("content", str_val("b")),
        ("tool_calls", arr(vec![obj(vec![("id", str_val("call-2"))])])),
    ])],
    false,
    Ok(None)
)]
#[case::new_content_null(
    vec![assistant_with_tool_calls()],
    vec![obj(vec![("role", str_val("assistant")), ("content", Value::Null)])],
    false,
    Ok(None)
)]
#[case::empty_new_content_string_still_merges(
    vec![assistant_with_tool_calls()],
    vec![assistant_answer("")],
    false,
    Ok(Some(obj(vec![
        ("role", str_val("assistant")),
        (
            "content",
            arr(vec![
                obj(vec![("type", str_val("text")), ("text", str_val("a"))]),
                obj(vec![("type", str_val("text")), ("text", str_val(""))]),
            ])
        ),
        ("tool_calls", arr(vec![obj(vec![("id", str_val("call-1"))])])),
    ])))
)]
#[case::malformed_content_block_raises(
    vec![obj(vec![
        ("role", str_val("assistant")),
        ("content", Value::Number(serde_json::Number::from(5_i64))),
        ("tool_calls", arr(vec![obj(vec![("id", str_val("call-1"))])])),
    ])],
    vec![assistant_answer("b")],
    false,
    Err(MergeError::InvalidContent)
)]
fn trailing_rejections(
    #[case] messages: Vec<Value>,
    #[case] new_messages: Vec<Value>,
    #[case] hosted_search: bool,
    #[case] expected: Result<Option<Value>, MergeError>,
) {
    assert_eq!(
        merge_trailing(&messages, &new_messages, hosted_search),
        expected
    );
}
