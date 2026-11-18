use betterlite_prompts::arguments::json::Json;

use crate::converse_messages::system::transform_system_message;

use super::support::{
    CLAUDE_3_5, CLAUDE_4_5, MODEL_COST, NOTE, assistant_turn, blocks, expected, messages, params,
    render, user_turn,
};

fn transform(history: &str, model: Option<&str>) -> (Json, Json) {
    let model_cost = params(MODEL_COST);
    let (kept, system) = transform_system_message(&messages(history), model, &model_cost)
        .expect("the messages transform");
    (render(&kept), blocks(&system))
}

const CACHE_POINT: &str = r#"{"cachePoint": {"type": "default"}}"#;

pub(super) fn system_text_cache_control() {
    string_content();
    list_content();
    model_capability();
    converted_systems_keep_their_control();
}

fn system_only(message: &str) -> Json {
    transform(
        &format!("[{message}, {}]", user_turn("question")),
        Some(CLAUDE_3_5),
    )
    .1
}

fn string_content() {
    let plain = r#"{"role": "system", "content": "rules"}"#;
    assert_eq!(system_only(plain), expected(r#"[{"text": "rules"}]"#));
    let cached =
        r#"{"role": "system", "content": "rules", "cache_control": {"type": "ephemeral"}}"#;
    assert_eq!(
        system_only(cached),
        expected(&format!(r#"[{{"text": "rules"}}, {CACHE_POINT}]"#))
    );
    let nulled = r#"{"role": "system", "content": "rules", "cache_control": null}"#;
    assert_eq!(system_only(nulled), expected(r#"[{"text": "rules"}]"#));
    let empty = r#"{"role": "system", "content": "", "cache_control": {"type": "ephemeral"}}"#;
    assert_eq!(system_only(empty), expected("[]"));
}

fn list_content() {
    let cached_parts = r#"{"role": "system", "content": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}, {"type": "text", "text": "b"}]}"#;
    let nulled_part =
        r#"{"role": "system", "content": [{"type": "text", "text": "a", "cache_control": null}]}"#;
    let message_level = r#"{"role": "system", "content": [{"type": "text", "text": "a"}], "cache_control": {"type": "ephemeral"}}"#;
    let no_text = r#"{"role": "system", "content": [{"type": "image"}], "cache_control": {"type": "ephemeral"}}"#;
    let cases = [
        (
            cached_parts,
            format!(r#"[{{"text": "a"}}, {CACHE_POINT}, {{"text": "b"}}]"#),
        ),
        (nulled_part, r#"[{"text": "a"}]"#.to_owned()),
        (message_level, r#"[{"text": "a"}]"#.to_owned()),
        (no_text, "[]".to_owned()),
    ];
    for (message, wanted) in cases {
        assert_eq!(system_only(message), expected(&wanted), "{message}");
    }
}

fn model_capability() {
    let cached =
        r#"{"role": "system", "content": "rules", "cache_control": {"type": "ephemeral"}}"#;
    for model in [Some("openai.gpt-5"), Some("anthropic.claude-no-cache")] {
        let (_, system) = transform(&format!("[{}, {}]", cached, user_turn("question")), model);
        assert_eq!(system, expected(r#"[{"text": "rules"}]"#), "{model:?}");
    }
    for model in [None, Some("anthropic.claude-unknown")] {
        let (_, system) = transform(&format!("[{}, {}]", cached, user_turn("question")), model);
        assert_eq!(
            system,
            expected(&format!(r#"[{{"text": "rules"}}, {CACHE_POINT}]"#)),
            "{model:?}"
        );
    }
    let ttl = r#"{"role": "system", "content": "rules", "cache_control": {"type": "ephemeral", "ttl": "1h"}}"#;
    let (_, system) = transform(
        &format!("[{}, {}]", ttl, user_turn("question")),
        Some(CLAUDE_4_5),
    );
    assert_eq!(
        system,
        expected(r#"[{"text": "rules"}, {"cachePoint": {"type": "default", "ttl": "1h"}}]"#)
    );
}

fn converted_systems_keep_their_control() {
    let history = format!(
        "[{}, {}, {}, {}]",
        r#"{"role": "system", "content": "top"}"#,
        user_turn("question"),
        r#"{"role": "system", "content": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}]}"#,
        assistant_turn("reply")
    );
    let (kept, system) = transform(&history, Some(CLAUDE_3_5));
    assert_eq!(system, expected(r#"[{"text": "top"}]"#));
    let expected_turn = format!(
        r#"{{"role": "user", "content": [{{"type": "text", "text": "{NOTE}"}}, {{"type": "text", "text": "a", "cache_control": {{"type": "ephemeral"}}}}]}}"#
    );
    assert_eq!(
        kept,
        expected(&format!(
            "[{}, {}, {}]",
            user_turn("question"),
            expected_turn,
            assistant_turn("reply")
        ))
    );
}
