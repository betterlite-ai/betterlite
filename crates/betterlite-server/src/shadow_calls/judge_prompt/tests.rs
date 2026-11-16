#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{
    MAX_JUDGE_PROMPT_CHARS, MAX_JUDGE_RESPONSE_CHARS, MAX_JUDGE_TOOL_DEFS_CHARS,
    PAIRWISE_JUDGE_SYSTEM_PROMPT, judge_user_prompt, tool_definitions_text,
};

fn tools(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn tool_with_description(name: &str, description: &str) -> Json {
    Json::List(vec![Json::Object(IndexMap::from([
        ("name".to_owned(), Json::Text(name.to_owned())),
        ("description".to_owned(), Json::Text(description.to_owned())),
    ]))])
}

#[test]
fn tool_definitions_render_a_function_tool_by_name_and_description() {
    let rendered = tool_definitions_text(&tools(
        r#"[{"type": "function", "function": {"name": "get_weather", "description": "Get the weather"}}]"#,
    ));

    assert_eq!(
        rendered,
        "Tools available to both responses:\n- get_weather: Get the weather"
    );
}

#[test]
fn tool_definitions_read_custom_then_the_entry_itself() {
    let rendered = tool_definitions_text(&tools(
        r#"[
            {"type": "custom", "custom": {"name": "run_code", "description": "Run code"}},
            {"name": "bare", "description": "Bare tool"},
            {"type": "function"}
        ]"#,
    ));

    assert_eq!(
        rendered,
        "Tools available to both responses:\n- run_code: Run code\n- bare: Bare tool\n- unnamed: no description"
    );
}

#[test]
fn tool_definitions_prefer_a_non_empty_function_over_custom() {
    let rendered = tool_definitions_text(&tools(
        r#"[{"function": {"name": "f", "description": "d"}, "custom": {"name": "c"}}]"#,
    ));
    let empty_function = tool_definitions_text(&tools(
        r#"[{"function": {}, "custom": {"name": "c", "description": "d"}}]"#,
    ));

    assert_eq!(rendered, "Tools available to both responses:\n- f: d");
    assert_eq!(empty_function, "Tools available to both responses:\n- c: d");
}

#[test]
fn tool_definitions_skip_string_entries_and_render_nothing_without_lines() {
    let mixed = tool_definitions_text(&tools(r#"["not a tool", {"name": "real"}]"#));

    assert_eq!(
        mixed,
        "Tools available to both responses:\n- real: no description"
    );
    assert_eq!(tool_definitions_text(&tools(r#"["only strings"]"#)), "");
    assert_eq!(tool_definitions_text(&tools("[]")), "");
}

#[rstest]
#[case::an_object(r#"{"name": "object"}"#)]
#[case::a_text(r#""text""#)]
#[case::null("null")]
#[case::a_number("7")]
fn tool_definitions_render_nothing_but_a_list(#[case] source: &str) {
    assert_eq!(tool_definitions_text(&tools(source)), "", "{source}");
}

#[test]
fn tool_definitions_default_empty_and_falsy_names_and_descriptions() {
    let rendered = tool_definitions_text(&tools(
        r#"[
            {"name": "", "description": ""},
            {"name": null},
            {"name": 0, "description": false},
            {"name": true, "description": 5}
        ]"#,
    ));

    assert_eq!(
        rendered,
        "Tools available to both responses:\n- unnamed: no description\n- unnamed: no description\n- unnamed: no description\n- True: 5"
    );
}

#[test]
fn tool_definitions_stop_at_the_cap_including_the_header() {
    let prefix = "Tools available to both responses:\n- big: ";
    let room = MAX_JUDGE_TOOL_DEFS_CHARS.saturating_sub(prefix.chars().count());

    let exact = tool_definitions_text(&tool_with_description("big", &"x".repeat(room)));
    let longer = tool_definitions_text(&tool_with_description(
        "big",
        &"x".repeat(room.saturating_add(1)),
    ));

    assert_eq!(exact, format!("{prefix}{}", "x".repeat(room)));
    assert_eq!(longer, format!("{prefix}{}", "x".repeat(room)));
    assert_eq!(longer.chars().count(), MAX_JUDGE_TOOL_DEFS_CHARS);
}

#[test]
fn judge_prompt_without_tools_has_no_prefix() {
    let prompt = judge_user_prompt("USER: hi", "first", "second", "");

    assert_eq!(
        prompt,
        "Conversation:\nUSER: hi\n\nResponse A:\nfirst\n\nResponse B:\nsecond\n\nWhich response is better?"
    );
}

#[test]
fn judge_prompt_carries_the_tool_text_and_two_newlines_before_the_conversation() {
    let prompt = judge_user_prompt(
        "USER: hi",
        "a",
        "b",
        "Tools available to both responses:\n- t: d",
    );

    assert_eq!(
        prompt,
        "Tools available to both responses:\n- t: d\n\nConversation:\nUSER: hi\n\nResponse A:\na\n\nResponse B:\nb\n\nWhich response is better?"
    );
}

#[test]
fn judge_prompt_caps_each_reply_and_gives_the_rest_to_the_conversation() {
    let reply = "r".repeat(9_000);
    let kept = "r".repeat(MAX_JUDGE_RESPONSE_CHARS);

    let prompt = judge_user_prompt("c", &reply, &reply, "");

    assert_eq!(
        prompt,
        format!(
            "Conversation:\nc\n\nResponse A:\n{kept}\n\nResponse B:\n{kept}\n\nWhich response is better?"
        )
    );
}

#[test]
fn judge_prompt_counts_the_tool_prefix_in_the_budget() {
    let conversation = "c".repeat(30_000);
    let tool_text = "t".repeat(100);
    let reply = "a".repeat(MAX_JUDGE_RESPONSE_CHARS);
    let budget = MAX_JUDGE_PROMPT_CHARS - MAX_JUDGE_RESPONSE_CHARS - 1 - 102;
    let tail: String = conversation
        .chars()
        .skip(conversation.chars().count().saturating_sub(budget))
        .collect();

    let prompt = judge_user_prompt(&conversation, &reply, "b", &tool_text);

    assert_eq!(
        prompt,
        format!(
            "{tool_text}\n\nConversation:\n{tail}\n\nResponse A:\n{reply}\n\nResponse B:\nb\n\nWhich response is better?"
        )
    );
}

#[test]
fn judge_prompt_keeps_the_end_of_a_long_conversation() {
    let conversation = format!("START{}END", "c".repeat(30_000));
    let budget = MAX_JUDGE_PROMPT_CHARS - 1 - 1;
    let tail: String = conversation
        .chars()
        .skip(conversation.chars().count().saturating_sub(budget))
        .collect();

    let prompt = judge_user_prompt(&conversation, "a", "b", "");

    assert_eq!(
        prompt,
        format!(
            "Conversation:\n{tail}\n\nResponse A:\na\n\nResponse B:\nb\n\nWhich response is better?"
        )
    );
    assert!(!prompt.contains("START"));
}

#[test]
fn judge_prompt_leaves_the_conversation_out_when_no_budget_remains() {
    let tool_text = "t".repeat(MAX_JUDGE_PROMPT_CHARS);
    let reply = "a".repeat(MAX_JUDGE_RESPONSE_CHARS);

    let prompt = judge_user_prompt("conversation", &reply, "", &tool_text);

    assert_eq!(
        prompt,
        format!(
            "{tool_text}\n\nConversation:\n\n\nResponse A:\n{reply}\n\nResponse B:\n\n\nWhich response is better?"
        )
    );
}

#[test]
fn judge_prompt_and_the_tool_cap_count_characters_not_bytes() {
    let reply = "\u{e9}".repeat(9_000);

    let prompt = judge_user_prompt("", &reply, "", "");

    assert!(prompt.contains(&"\u{e9}".repeat(MAX_JUDGE_RESPONSE_CHARS)));
    assert!(!prompt.contains(&"\u{e9}".repeat(MAX_JUDGE_RESPONSE_CHARS + 1)));

    let header = "Tools available to both responses:\n- big: ";
    let capped = tool_definitions_text(&tool_with_description("big", &"\u{e9}".repeat(3_000)));
    let room = MAX_JUDGE_TOOL_DEFS_CHARS.saturating_sub(header.chars().count());

    assert_eq!(capped, format!("{header}{}", "\u{e9}".repeat(room)));
}

#[test]
fn judge_prompt_counts_non_ascii_characters_in_the_budget() {
    let conversation = "\u{e9}".repeat(30_000);
    let tool_text = "\u{fc}".repeat(100);
    let reply = "\u{436}".repeat(MAX_JUDGE_RESPONSE_CHARS);
    let budget = MAX_JUDGE_PROMPT_CHARS - MAX_JUDGE_RESPONSE_CHARS - 1 - 102;
    let tail = "\u{e9}".repeat(budget);

    let prompt = judge_user_prompt(&conversation, &reply, "b", &tool_text);

    assert_eq!(
        prompt,
        format!(
            "{tool_text}\n\nConversation:\n{tail}\n\nResponse A:\n{reply}\n\nResponse B:\nb\n\nWhich response is better?"
        )
    );
}

const LITELLM_SYSTEM_PROMPT: &str = concat!(
    "You are an impartial quality judge comparing two responses to the same conversation.\n",
    "\n",
    "The responses are labeled A and B in random order. You do not know which system produced which.\n",
    "\n",
    "A response may be prose, or a tool call shown as `[tool call] name(arguments)` if the\n",
    "assistant chose to act instead of answering directly. A tool call is not a defect: judge\n",
    "whether calling that tool was the right response to the conversation, the same as you\n",
    "would judge prose.\n",
    "\n",
    "Criteria: correctness, completeness, clarity, conciseness.\n",
    "\n",
    "Return ONLY valid JSON in this exact format, no other text:\n",
    "{\n",
    "  \"preference\": \"A\" | \"B\" | \"tie\",\n",
    "  \"confidence\": <0.0 to 1.0>\n",
    "}",
);

#[test]
fn the_system_prompt_keeps_litellms_text_and_two_line_json_shape() {
    assert_eq!(PAIRWISE_JUDGE_SYSTEM_PROMPT, LITELLM_SYSTEM_PROMPT);
}
