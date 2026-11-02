#![cfg(test)]

use super::content::{Content, ReasoningText, ThinkingBlocks};
use super::record::{ChatRecord, Record};
use super::trailing::{TrailingInput, TrailingRecord, merge_trailing as merge_trailing_records};
use super::{MergeError, Reasoning as _, Role, merge_reasoning};
use rstest::rstest;
use serde_json::{Map, Value};

fn merge_trailing(
    messages: &[Value],
    new_messages: &[Value],
    hosted_search: bool,
) -> Result<Option<Value>, MergeError> {
    let typed_messages: Vec<TrailingInput> = messages
        .iter()
        .cloned()
        .map(TrailingInput::from_value)
        .collect::<Result<_, _>>()?;
    let typed_new_messages: Vec<TrailingInput> = new_messages
        .iter()
        .cloned()
        .map(TrailingInput::from_value)
        .collect::<Result<_, _>>()?;
    let merged = merge_trailing_records(&typed_messages, &typed_new_messages, hosted_search)?;
    Ok(merged.map(|record| serde_json::to_value(record).expect("trailing record serializes")))
}

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
fn reasoning_missing_content_key_stays_missing() {
    let output = merge_dicts(vec![
        obj(vec![("role", str_val("tool")), ("tool_calls", Value::Null)]),
        obj(vec![("role", str_val("user")), ("content", str_val("q"))]),
    ]);
    assert_eq!(
        output,
        vec![
            obj(vec![("role", str_val("tool")), ("tool_calls", Value::Null)]),
            obj(vec![("role", str_val("user")), ("content", str_val("q"))]),
        ]
    );
}

#[test]
fn reasoning_boundaries_typed_records() {
    let before = Record::Attribute(ChatRecord {
        role: Role::ASSISTANT,
        content: None,
        reasoning_content: Some(Some(ReasoningText::Text("a".to_owned()))),
        thinking_blocks: None,
        tool_calls: None,
        extra: Map::new(),
    });
    let user = Record::Attribute(ChatRecord {
        role: Role::USER,
        content: Some(Some(Content::Text("q".to_owned()))),
        reasoning_content: None,
        thinking_blocks: None,
        tool_calls: None,
        extra: Map::new(),
    });
    let after = Record::Attribute(ChatRecord {
        role: Role::ASSISTANT,
        content: None,
        reasoning_content: None,
        thinking_blocks: Some(Some(ThinkingBlocks::List(vec![str_val("B")]))),
        tool_calls: None,
        extra: Map::new(),
    });
    let mut records = vec![before, user.clone(), after];
    let output = merge_reasoning(&mut records);
    // Both pending turns rebuild as the dict shape, even though their source records were attribute-typed.
    assert_eq!(
        output,
        vec![
            Record::Dict(ChatRecord::blank(Some("a".to_owned()), None)),
            user,
            Record::Dict(ChatRecord::blank(None, Some(vec![str_val("B")]))),
        ]
    );
}

#[test]
fn reasoning_merges_into_typed_target() {
    let pending = Record::Attribute(ChatRecord {
        role: Role::ASSISTANT,
        content: None,
        reasoning_content: Some(Some(ReasoningText::Text("a".to_owned()))),
        thinking_blocks: Some(Some(ThinkingBlocks::List(vec![str_val("A")]))),
        tool_calls: None,
        extra: Map::new(),
    });
    let target = Record::Attribute(ChatRecord {
        role: Role::ASSISTANT,
        content: Some(Some(Content::Text("answer".to_owned()))),
        reasoning_content: Some(Some(ReasoningText::Text("c".to_owned()))),
        thinking_blocks: Some(Some(ThinkingBlocks::List(vec![str_val("C")]))),
        tool_calls: None,
        extra: Map::new(),
    });
    let mut records = vec![pending, target];
    let output = merge_reasoning(&mut records);
    let expected = Record::Attribute(ChatRecord {
        role: Role::ASSISTANT,
        content: Some(Some(Content::Text("answer".to_owned()))),
        reasoning_content: Some(Some(ReasoningText::Text("a\nc".to_owned()))),
        thinking_blocks: Some(Some(ThinkingBlocks::List(vec![str_val("A"), str_val("C")]))),
        tool_calls: None,
        extra: Map::new(),
    });
    assert_eq!(output, vec![expected.clone()]);
    // The target element in the original slice is mutated in place, not left as a pending-free clone.
    assert_eq!(records[1], expected);
}

#[test]
fn reasoning_mixed_dict_and_attribute_records() {
    let pending = Record::Dict(ChatRecord {
        role: Role::ASSISTANT,
        content: Some(None),
        reasoning_content: Some(Some(ReasoningText::Text("a".to_owned()))),
        thinking_blocks: Some(Some(ThinkingBlocks::List(vec![str_val("A")]))),
        tool_calls: None,
        extra: Map::new(),
    });
    let target = Record::Attribute(ChatRecord {
        role: Role::ASSISTANT,
        content: Some(Some(Content::Text("answer".to_owned()))),
        reasoning_content: None,
        thinking_blocks: None,
        tool_calls: Some(Some(vec![str_val("call-1")])),
        extra: Map::new(),
    });
    let mut records = vec![pending, target];
    let output = merge_reasoning(&mut records);
    let Record::Attribute(merged) = &output[0] else {
        panic!(
            "a dict-kind pending turn merges into an attribute-kind target without changing its kind"
        );
    };
    assert_eq!(merged.text(), Some("a"));
    assert_eq!(merged.blocks(), Some(&[str_val("A")][..]));
    assert_eq!(
        merged.content,
        Some(Some(Content::Text("answer".to_owned())))
    );
}

#[test]
fn attribute_record_list_content_is_a_merge_target_not_pending() {
    let record = ChatRecord {
        role: Role::ASSISTANT,
        content: Some(Some(Content::Blocks(vec![str_val("block")]))),
        reasoning_content: Some(Some(ReasoningText::Other(Value::Number(
            serde_json::Number::from(5_i64),
        )))),
        thinking_blocks: None,
        tool_calls: None,
        extra: Map::new(),
    };
    // List content is not null, so this is an assistant target, not a reasoning-only pending turn.
    assert!(!record.is_pending());
    // A non-string reasoning_content is ignored for merge purposes, not rejected.
    assert_eq!(record.text(), None);
}

#[test]
fn attribute_record_extra_fields_survive_merge() {
    let pending = ChatRecord {
        role: Role::ASSISTANT,
        content: Some(None),
        reasoning_content: Some(Some(ReasoningText::Text("a".to_owned()))),
        thinking_blocks: None,
        tool_calls: None,
        extra: Map::new(),
    };
    let mut extra = Map::new();
    extra.insert("name".to_owned(), str_val("carried"));
    extra.insert("tool_call_id".to_owned(), str_val("call-1"));
    let target = ChatRecord {
        role: Role::ASSISTANT,
        content: Some(Some(Content::Text("answer".to_owned()))),
        reasoning_content: None,
        thinking_blocks: None,
        tool_calls: None,
        extra,
    };
    let mut records = vec![pending, target.clone()];
    let output = merge_reasoning(&mut records);
    assert_eq!(output[0].extra, target.extra);
}

#[test]
fn merge_reasoning_mutates_original_target_record() {
    let mut records = vec![
        ChatRecord {
            role: Role::ASSISTANT,
            content: Some(None),
            reasoning_content: Some(Some(ReasoningText::Text("a".to_owned()))),
            thinking_blocks: None,
            tool_calls: None,
            extra: Map::new(),
        },
        ChatRecord {
            role: Role::ASSISTANT,
            content: Some(Some(Content::Text("answer".to_owned()))),
            reasoning_content: Some(Some(ReasoningText::Text("c".to_owned()))),
            thinking_blocks: None,
            tool_calls: None,
            extra: Map::new(),
        },
    ];
    let output = merge_reasoning(&mut records);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].text(), Some("a\nc"));
    assert_eq!(output, vec![records[1].clone()]);
}

#[test]
fn standalone_builder_with_no_reasoning_is_content_null_only() {
    let built = ChatRecord::blank(None, None);
    assert_eq!(
        built,
        ChatRecord {
            role: Role::ASSISTANT,
            content: Some(None),
            reasoning_content: None,
            thinking_blocks: None,
            tool_calls: None,
            extra: Map::new(),
        }
    );
}

#[test]
fn standalone_builder_omits_empty_reasoning_values() {
    let built = ChatRecord::blank(Some(String::new()), Some(Vec::new()));
    assert_eq!(built, ChatRecord::blank(None, None));
}

#[test]
fn roleless_and_unknown_records_are_non_assistant_boundaries() {
    let output = merge_dicts(vec![
        obj(vec![
            ("role", str_val("assistant")),
            ("content", Value::Null),
            ("reasoning_content", str_val("before")),
        ]),
        obj(vec![("tool_calls", arr(vec![str_val("call")]))]),
        obj(vec![
            ("role", str_val("vendor-role")),
            ("content", str_val("x")),
        ]),
    ]);
    assert_eq!(
        output,
        vec![
            obj(vec![
                ("role", str_val("assistant")),
                ("content", Value::Null),
                ("reasoning_content", str_val("before"))
            ]),
            obj(vec![("tool_calls", arr(vec![str_val("call")]))]),
            obj(vec![
                ("role", str_val("vendor-role")),
                ("content", str_val("x"))
            ]),
        ]
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

// serde_json::Error carries no PartialEq, so a rejection names the variant rather than its cause.
#[derive(Debug)]
enum Expectation {
    Merged(Option<Value>),
    InvalidRecord,
    InvalidContent,
    InvalidNewMessage,
}

fn assert_merge(result: Result<Option<Value>, MergeError>, expected: Expectation) {
    match (result, expected) {
        (Ok(value), Expectation::Merged(wanted)) => assert_eq!(value, wanted),
        (Err(MergeError::InvalidRecord { .. }), Expectation::InvalidRecord)
        | (Err(MergeError::InvalidContent), Expectation::InvalidContent)
        | (Err(MergeError::InvalidNewMessage), Expectation::InvalidNewMessage) => {}
        (other, other_expected) => panic!("unexpected {other:?} for {other_expected:?}"),
    }
}

#[rstest]
#[case::no_prior_messages(vec![], vec![], false, Expectation::Merged(None))]
#[case::zero_new_messages(vec![assistant_with_tool_calls()], vec![], false, Expectation::Merged(None))]
#[case::two_new_messages(
    vec![assistant_with_tool_calls()],
    vec![assistant_answer("b"), assistant_answer("c")],
    false,
    Expectation::Merged(None)
)]
#[case::typed_non_dict_last_message(
    vec![arr(vec![str_val("not"), str_val("a"), str_val("dict")])],
    vec![assistant_answer("b")],
    false,
    Expectation::Merged(None)
)]
#[case::new_message_not_dict_raises(
    vec![assistant_with_tool_calls()],
    vec![arr(vec![str_val("not"), str_val("a"), str_val("dict")])],
    false,
    Expectation::InvalidNewMessage
)]
#[case::last_message_malformed_tool_calls_raises(
    vec![obj(vec![
        ("role", str_val("assistant")),
        ("content", str_val("a")),
        ("tool_calls", Value::Bool(true)),
    ])],
    vec![assistant_answer("b")],
    false,
    Expectation::InvalidRecord
)]
#[case::last_message_not_assistant(
    vec![obj(vec![
        ("role", str_val("user")),
        ("content", str_val("a")),
        ("tool_calls", arr(vec![obj(vec![("id", str_val("call-1"))])])),
    ])],
    vec![assistant_answer("b")],
    false,
    Expectation::Merged(None)
)]
#[case::new_message_not_assistant(
    vec![assistant_with_tool_calls()],
    vec![obj(vec![("role", str_val("user")), ("content", str_val("b"))])],
    false,
    Expectation::Merged(None)
)]
#[case::no_prior_tool_calls_without_hosted_search(
    vec![assistant_answer("a")],
    vec![assistant_answer("b")],
    false,
    Expectation::Merged(None)
)]
#[case::truthy_new_tool_calls(
    vec![assistant_with_tool_calls()],
    vec![obj(vec![
        ("role", str_val("assistant")),
        ("content", str_val("b")),
        ("tool_calls", arr(vec![obj(vec![("id", str_val("call-2"))])])),
    ])],
    false,
    Expectation::Merged(None)
)]
#[case::new_content_null(
    vec![assistant_with_tool_calls()],
    vec![obj(vec![("role", str_val("assistant")), ("content", Value::Null)])],
    false,
    Expectation::Merged(None)
)]
#[case::empty_new_content_string_still_merges(
    vec![assistant_with_tool_calls()],
    vec![assistant_answer("")],
    false,
    Expectation::Merged(Some(obj(vec![
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
    Expectation::InvalidContent
)]
fn trailing_rejections(
    #[case] messages: Vec<Value>,
    #[case] new_messages: Vec<Value>,
    #[case] hosted_search: bool,
    #[case] expected: Expectation,
) {
    assert_merge(
        merge_trailing(&messages, &new_messages, hosted_search),
        expected,
    );
}

#[test]
fn trailing_assistant_hosted_search_preserves_explicit_null_tool_calls() {
    let output = merge_trailing(
        &[obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("a")),
            ("tool_calls", Value::Null),
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
            ("tool_calls", Value::Null),
        ]))
    );
}

#[test]
fn non_list_thinking_blocks_is_preserved_on_dict_record_without_merging() {
    let output = merge_dicts(vec![
        obj(vec![
            ("role", str_val("assistant")),
            ("content", Value::Null),
            ("reasoning_content", str_val("a")),
        ]),
        obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("answer")),
            (
                "thinking_blocks",
                obj(vec![("note", str_val("not-a-list"))]),
            ),
        ]),
    ]);
    // Pending text still merges in; the target's own non-list thinking_blocks is untouched, not rejected.
    assert_eq!(
        output,
        vec![obj(vec![
            ("role", str_val("assistant")),
            ("content", str_val("answer")),
            ("reasoning_content", str_val("a")),
            (
                "thinking_blocks",
                obj(vec![("note", str_val("not-a-list"))])
            ),
        ])]
    );
}

#[test]
fn non_list_thinking_blocks_is_not_pending_on_attribute_record() {
    let record = Record::Attribute(ChatRecord {
        role: Role::ASSISTANT,
        content: Some(None),
        reasoning_content: None,
        thinking_blocks: Some(Some(ThinkingBlocks::Other(obj(vec![(
            "note",
            str_val("not-a-list"),
        )])))),
        tool_calls: None,
        extra: Map::new(),
    });
    // content=null but a non-list thinking_blocks and no reasoning text leave nothing to accumulate.
    assert!(!record.is_pending());
    assert_eq!(record.blocks(), None);
}

#[test]
fn tool_calls_wrong_type_is_rejected() {
    let value = obj(vec![
        ("role", str_val("assistant")),
        ("content", str_val("a")),
        ("tool_calls", Value::Bool(true)),
    ]);
    serde_json::from_value::<ChatRecord>(value.clone()).unwrap_err();
    serde_json::from_value::<TrailingRecord>(value).unwrap_err();
}
