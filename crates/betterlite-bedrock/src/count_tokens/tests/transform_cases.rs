use crate::count_tokens::converse::{
    to_bedrock_tool_spec, to_converse_message, transform_system, transform_tools,
};
use crate::count_tokens::error::CountTokensError;
use crate::count_tokens::validate::validate_count_tokens_request;
use crate::count_tokens::{InputType, detect_input_type, transform_request};

use super::support::{
    assert_envelope, assert_invoke_body, assert_json, request, value, with_model,
};

pub(super) fn typed_block_invoke_model() {
    let body = with_model(
        r#""max_tokens": 256, "messages": [{"content": [{"text": "Hello!", "type": "text"}], "role": "user"}]"#,
    );
    assert_eq!(detect_input_type(&body), InputType::InvokeModel);
    assert_invoke_body(
        &transform_request(&body),
        r#"{"max_tokens": 256, "messages": [{"content": [{"text": "Hello!", "type": "text"}], "role": "user"}], "anthropic_version": "bedrock-2023-05-31"}"#,
    );
}

pub(super) fn untyped_messages_converse() {
    let plain = with_model(r#""messages": [{"content": [{"text": "hi"}], "role": "user"}]"#);
    assert_eq!(detect_input_type(&plain), InputType::Converse);
    assert_eq!(validate_count_tokens_request(&plain), Ok(()));
    assert_envelope(
        &transform_request(&plain),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#,
    );
    let empty = with_model(r#""messages": []"#);
    assert_eq!(detect_input_type(&empty), InputType::Converse);
    assert_eq!(
        validate_count_tokens_request(&empty),
        Err(CountTokensError::MessagesRequired)
    );
    let malformed = with_model(r#""messages": [42]"#);
    assert_eq!(detect_input_type(&malformed), InputType::Converse);
    assert_eq!(
        validate_count_tokens_request(&malformed),
        Err(CountTokensError::MessageNotObject { index: 0 })
    );
    let untyped_blocks = with_model(
        r#""messages": [{"content": "plain", "role": "user"}, {"content": [{"toolUse": {"name": "t"}}], "role": "assistant"}]"#,
    );
    assert_eq!(detect_input_type(&untyped_blocks), InputType::Converse);
    assert_envelope(
        &transform_request(&untyped_blocks),
        r#"{"input": {"converse": {"messages": [
            {"role": "user", "content": [{"text": "plain"}]},
            {"role": "assistant", "content": [{"toolUse": {"name": "t"}}]}
        ]}}}"#,
    );
    non_list_messages();
}

fn non_list_messages() {
    let prompt = with_model(r#""messages": "prompt""#);
    assert_eq!(detect_input_type(&prompt), InputType::InvokeModel);
    assert_invoke_body(
        &transform_request(&prompt),
        r#"{"messages": "prompt", "anthropic_version": "bedrock-2023-05-31", "max_tokens": 1024}"#,
    );
    let mapping = with_model(r#""messages": {"content": "hi", "role": "user"}"#);
    assert_eq!(detect_input_type(&mapping), InputType::InvokeModel);
    assert_invoke_body(
        &transform_request(&mapping),
        r#"{"messages": {"content": "hi", "role": "user"}, "anthropic_version": "bedrock-2023-05-31", "max_tokens": 1024}"#,
    );
    let only_model = request(r#"{"model": "anthropic.claude-3-5-sonnet-20241022-v2:0"}"#);
    assert_eq!(detect_input_type(&only_model), InputType::InvokeModel);
    assert_eq!(
        validate_count_tokens_request(&only_model),
        Err(CountTokensError::ContentRequired)
    );
    let with_content =
        request(r#"{"extra": null, "model": "anthropic.claude-3-5-sonnet-20241022-v2:0"}"#);
    assert_eq!(validate_count_tokens_request(&with_content), Ok(()));
}

pub(super) fn tool_name_schema() {
    let body = with_model(
        r#""messages": [{"content": "hi", "role": "user"}], "tools": [
            {"name": "my-tool", "description": "keeps", "input_schema": {"type": "object"}},
            {"name": "9lives"},
            {},
            {"name": 123},
            {"name": "a b.c/d", "description": "d"},
            {"name": "1 tool-name/with spaces_and_more_characters_that_go_past_the_limit_1234567890"},
            {"name": "\u00e9"},
            "junk",
            5
        ]"#,
    );
    assert_envelope(
        &transform_request(&body),
        r#"{"input": {"converse": {
            "messages": [{"role": "user", "content": [{"text": "hi"}]}],
            "toolConfig": {"tools": [
                {"toolSpec": {"name": "my_tool", "description": "keeps", "inputSchema": {"json": {"type": "object"}}}},
                {"toolSpec": {"name": "t_9lives", "description": "t_9lives", "inputSchema": {"json": {"type": "object", "properties": {}}}}},
                {"toolSpec": {"name": "", "description": "", "inputSchema": {"json": {"type": "object", "properties": {}}}}},
                {"toolSpec": {"name": "", "description": "", "inputSchema": {"json": {"type": "object", "properties": {}}}}},
                {"toolSpec": {"name": "a_b_c_d", "description": "d", "inputSchema": {"json": {"type": "object", "properties": {}}}}},
                {"toolSpec": {"name": "t_1_tool_name_with_spaces_and_more_characters_that_go_past_the_l", "description": "t_1_tool_name_with_spaces_and_more_characters_that_go_past_the_l", "inputSchema": {"json": {"type": "object", "properties": {}}}}},
                {"toolSpec": {"name": "t__", "description": "t__", "inputSchema": {"json": {"type": "object", "properties": {}}}}},
                {"toolSpec": {"name": "", "description": "", "inputSchema": {"json": {"type": "object", "properties": {}}}}},
                {"toolSpec": {"name": "", "description": "", "inputSchema": {"json": {"type": "object", "properties": {}}}}}
            ]}
        }}}"#,
    );
    tool_name_defaults();
}

fn tool_name_defaults() {
    let default_schema = r#"{"type": "object", "properties": {}}"#;
    assert_json(
        &to_bedrock_tool_spec(&value(r#"{"name": "t", "description": ""}"#)),
        &format!(
            r#"{{"toolSpec": {{"name": "t", "description": "", "inputSchema": {{"json": {default_schema}}}}}}}"#
        ),
    );
    assert_json(
        &to_bedrock_tool_spec(&value(
            r#"{"name": "t", "description": null, "input_schema": null}"#,
        )),
        &format!(
            r#"{{"toolSpec": {{"name": "t", "description": "t", "inputSchema": {{"json": {default_schema}}}}}}}"#
        ),
    );
    assert_json(
        &to_bedrock_tool_spec(&value("7")),
        &format!(
            r#"{{"toolSpec": {{"name": "", "description": "", "inputSchema": {{"json": {default_schema}}}}}}}"#
        ),
    );
}

pub(super) fn invoke_body_python_json_spacing() {
    let body = with_model(
        r#""messages": [{"content": [{"text": "h\u00e9llo \ud83d\ude00", "type": "text"}], "role": "user"}],
        "metadata": {"na\u00efve": ["caff\u00e9", "tab\there", "line\nbreak", "quote\"d", "back\\slash", "\u007f"]},
        "scale": 1.0,
        "temperature": 0.7"#,
    );
    assert_invoke_body(
        &transform_request(&body),
        r#"{"messages": [{"content": [{"text": "h\u00e9llo \ud83d\ude00", "type": "text"}], "role": "user"}], "metadata": {"na\u00efve": ["caff\u00e9", "tab\there", "line\nbreak", "quote\"d", "back\\slash", "\u007f"]}, "scale": 1.0, "temperature": 0.7, "anthropic_version": "bedrock-2023-05-31", "max_tokens": 1024}"#,
    );
}

pub(super) fn invoke_body_defaults() {
    let defaults = with_model(
        r#""messages": [{"content": [{"text": "hi", "type": "text"}], "role": "user"}]"#,
    );
    assert_invoke_body(
        &transform_request(&defaults),
        r#"{"messages": [{"content": [{"text": "hi", "type": "text"}], "role": "user"}], "anthropic_version": "bedrock-2023-05-31", "max_tokens": 1024}"#,
    );
    let explicit = with_model(
        r#""anthropic_version": "custom-version", "max_tokens": 5, "messages": [{"content": [{"text": "hi", "type": "text"}], "role": "user"}]"#,
    );
    assert_invoke_body(
        &transform_request(&explicit),
        r#"{"anthropic_version": "custom-version", "max_tokens": 5, "messages": [{"content": [{"text": "hi", "type": "text"}], "role": "user"}]}"#,
    );
    let nulls = with_model(
        r#""anthropic_version": null, "max_tokens": null, "messages": [{"content": [{"text": "hi", "type": "text"}], "role": "user"}]"#,
    );
    assert_invoke_body(
        &transform_request(&nulls),
        r#"{"anthropic_version": null, "max_tokens": null, "messages": [{"content": [{"text": "hi", "type": "text"}], "role": "user"}]}"#,
    );
    prompt_only_bodies();
}

fn prompt_only_bodies() {
    let prompt = with_model(r#""prompt": "\n\nHuman: hi\n\nAssistant:""#);
    assert_invoke_body(
        &transform_request(&prompt),
        r#"{"prompt": "\n\nHuman: hi\n\nAssistant:"}"#,
    );
    let null_messages = with_model(r#""messages": null"#);
    assert_invoke_body(
        &transform_request(&null_messages),
        r#"{"messages": null, "anthropic_version": "bedrock-2023-05-31", "max_tokens": 1024}"#,
    );
}

pub(super) fn converse_content_system() {
    let text = with_model(r#""messages": [{"content": "Hello", "role": "user"}]"#);
    assert_envelope(
        &transform_request(&text),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "Hello"}]}]}}}"#,
    );
    let list = with_model(
        r#""messages": [{"content": [{"text": "a"}, {"toolUse": {"name": "t"}}, "bare", 5], "role": "assistant"}]"#,
    );
    assert_envelope(
        &transform_request(&list),
        r#"{"input": {"converse": {"messages": [{"role": "assistant", "content": [{"text": "a"}, {"toolUse": {"name": "t"}}, "bare", 5]}]}}}"#,
    );
    let number = with_model(r#""messages": [{"content": 5, "role": "user"}]"#);
    assert_envelope(
        &transform_request(&number),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": []}]}}}"#,
    );
    let null_content = with_model(r#""messages": [{"content": null, "role": "user"}]"#);
    assert_envelope(
        &transform_request(&null_content),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": []}]}}}"#,
    );
    let empty_content = with_model(r#""messages": [{"content": [], "role": "user"}]"#);
    assert_envelope(
        &transform_request(&empty_content),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": []}]}}}"#,
    );
    converse_system_blocks();
}

fn converse_system_blocks() {
    let string_system =
        with_model(r#""messages": [{"content": "hi", "role": "user"}], "system": "be nice""#);
    assert_envelope(
        &transform_request(&string_system),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}], "system": [{"text": "be nice"}]}}}"#,
    );
    let block_system = with_model(
        r#""messages": [{"content": "hi", "role": "user"}], "system": [{"text": "a"}, {"type": "text", "text": "b"}, {"other": true}, "skip", 5]"#,
    );
    assert_envelope(
        &transform_request(&block_system),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}], "system": [{"text": "a"}, {"text": "b"}, {"text": ""}]}}}"#,
    );
    let ignored =
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#;
    let number_system =
        with_model(r#""messages": [{"content": "hi", "role": "user"}], "system": 5"#);
    assert_envelope(&transform_request(&number_system), ignored);
    let null_system =
        with_model(r#""messages": [{"content": "hi", "role": "user"}], "system": null"#);
    assert_envelope(&transform_request(&null_system), ignored);
    let empty_system =
        with_model(r#""messages": [{"content": "hi", "role": "user"}], "system": []"#);
    assert_envelope(&transform_request(&empty_system), ignored);
    converse_block_defaults();
}

fn converse_block_defaults() {
    assert_json(
        &to_converse_message(&value(r#"{"role": "user"}"#)),
        r#"{"role": "user", "content": [{"text": ""}]}"#,
    );
    assert_json(
        &to_converse_message(&value("7")),
        r#"{"role": null, "content": [{"text": ""}]}"#,
    );
    let blocks = value(r#"[{"text": null}, {"text": "x"}, {"none": true}, "skip"]"#);
    assert_json(
        &transform_system(Some(&blocks)),
        r#"[{"text": ""}, {"text": "x"}, {"text": ""}]"#,
    );
}

pub(super) fn empty_tools_omission() {
    let absent = with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": false"#);
    assert_envelope(
        &transform_request(&absent),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#,
    );
    let text =
        with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": "not-a-list""#);
    assert_envelope(
        &transform_request(&text),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}], "toolConfig": {"tools": []}}}}"#,
    );
    let empty_list = with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": []"#);
    assert_envelope(
        &transform_request(&empty_list),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#,
    );
    let null_tools =
        with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": null"#);
    assert_envelope(
        &transform_request(&null_tools),
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#,
    );
    omitted_tools();
}

fn omitted_tools() {
    let ignored =
        r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#;
    let empty_object =
        with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": {}"#);
    assert_envelope(&transform_request(&empty_object), ignored);
    let empty_text = with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": """#);
    assert_envelope(&transform_request(&empty_text), ignored);
    let zero = with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": 0"#);
    assert_envelope(&transform_request(&zero), ignored);
    assert_json(&transform_tools(Some(&value("false"))), "null");
    assert_json(
        &transform_tools(Some(&value(r#"["x"]"#))),
        r#"{"tools": [{"toolSpec": {"name": "", "description": "", "inputSchema": {"json": {"type": "object", "properties": {}}}}}]}"#,
    );
    non_list_tools();
}

fn non_list_tools() {
    let empty_tools = r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}], "toolConfig": {"tools": []}}}}"#;
    let object_tools =
        with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": {"a": 1}"#);
    assert_envelope(&transform_request(&object_tools), empty_tools);
    let number_tools = with_model(r#""messages": [{"content": "hi", "role": "user"}], "tools": 1"#);
    assert_envelope(&transform_request(&number_tools), empty_tools);
}
