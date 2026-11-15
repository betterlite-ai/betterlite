use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};
use rstest::rstest;

use super::{
    TAIL_SENTINEL, assert_strip, body, chat, default_markers, encrypted_item, kind_part,
    last_ask_index, one_pair, resolved_ask, resolver, text, text_part,
};
use crate::classifier_message_text::asks::{
    current_ask_and_system_prompt, human_asks_newest_first, newest_turn_ask,
    newest_turn_is_human_ask,
};
use crate::classifier_message_text::encrypted::encrypted_classifier_task;
use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::image::inline_image_part;
use crate::classifier_message_text::markers::{MarkerPair, is_codex_user_agent};
use crate::classifier_message_text::plan::{PLAN_MODE_TOOL_NAME, matched_plan_mode_sentinel};
use crate::classifier_message_text::text::{message_text, strip_reminder_blocks};

fn assert_last_ask(messages: &[ChatMessage], expected: Option<usize>) {
    assert_eq!(last_ask_index(messages, default_markers()), expected);
}

fn assert_plan_pattern(messages: &[ChatMessage], expected: &str) {
    let result = matched_plan_mode_sentinel(None, Some(messages), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(expected));
}

fn assert_encrypted_input(
    input: Option<Json>,
    resolved: Vec<Vec<Json>>,
    expected: Result<Option<Json>, ClassifierTextError>,
) {
    let task =
        encrypted_classifier_task(input.as_ref(), false, resolver(resolved), default_markers());
    assert_eq!(task, expected);
}

fn assert_encrypted_absent(input: Json, resolved: Vec<Vec<Json>>) {
    assert_encrypted_input(Some(input), resolved, Ok(None));
}

fn assert_encrypted_absent_with_ask(item: Json) {
    assert_encrypted_absent(item, vec![resolved_ask()]);
}

fn assert_encrypted_input_error(item: Json, expected: ClassifierTextError) {
    assert_encrypted_input(Some(item), vec![resolved_ask()], Err(expected));
}

fn assert_empty_message_list() {
    let messages: [ChatMessage; 0] = [];
    assert_eq!(
        human_asks_newest_first(&messages, default_markers()).unwrap(),
        Vec::new()
    );
    assert_eq!(newest_turn_ask(&messages, default_markers()).unwrap(), None);
    assert_last_ask(&messages, None);
    assert_eq!(
        current_ask_and_system_prompt(&messages, default_markers()).unwrap(),
        (None, None)
    );
}

fn assert_blank_text_is_not_ask() {
    let messages = [text(Role::User, "   ")];
    assert_eq!(newest_turn_ask(&messages, default_markers()).unwrap(), None);
    assert_eq!(
        human_asks_newest_first(&messages, default_markers())
            .unwrap()
            .len(),
        0
    );
}

fn assert_empty_markers_strip_nothing() {
    assert_eq!(strip_reminder_blocks("  plain  ", &[]), "plain");
}

fn assert_marker_at_both_ends() {
    assert_strip(
        "<system-reminder>a</system-reminder>keep<system-reminder>b</system-reminder>",
        "keep",
    );
}

fn assert_overlapping_marker_pairs() {
    let markers = one_pair("<system-reminder>", "</system-reminder>");
    assert_eq!(
        strip_reminder_blocks("<system-reminder>a</system-reminder>", &markers),
        ""
    );
    let crossed = [
        MarkerPair {
            open: "<a>",
            close: "</b>",
        },
        MarkerPair {
            open: "<b>",
            close: "</a>",
        },
    ];
    assert_eq!(
        strip_reminder_blocks("x <a>1<b>2</b>3</a> y", &crossed),
        "x y"
    );
}

fn assert_text_part_not_string_names_index() {
    let part = kind_part("text", "text", serde_json::json!(7));
    let error =
        message_text(Some(&MessageContent::Parts(vec![text_part("ok"), part]))).unwrap_err();
    assert_eq!(error, ClassifierTextError::PartTextNotString { part: 1 });
}

fn assert_image_url_not_string_names_field() {
    let part = kind_part("image_url", "image_url", serde_json::json!({"url": 7}));
    let error = inline_image_part(&part).unwrap_err();
    assert_eq!(error, ClassifierTextError::ImageUrlNotString);
}

fn assert_json_text_part_null_names_index() {
    let content = body(serde_json::json!([
        {"type": "text", "text": "fine"},
        {"type": "text", "text": null}
    ]));
    let error = message_text(Some(&content)).unwrap_err();
    assert_eq!(error, ClassifierTextError::PartTextNotString { part: 1 });
}

fn assert_last_ask_index_with_system_tail() {
    let messages = [
        text(Role::User, "ask"),
        text(Role::System, "trailing system"),
    ];
    assert_last_ask(&messages, Some(0));
}

fn assert_last_ask_index_not_zero() {
    let messages = [
        text(Role::User, "<system-reminder>plumbing</system-reminder>"),
        text(Role::User, "the ask"),
    ];
    assert_last_ask(&messages, Some(1));
}

fn assert_strip_block_at_end() {
    assert_strip("keep <system-reminder></system-reminder>", "keep");
}

fn assert_newest_turn_no_messages() {
    assert!(!newest_turn_is_human_ask(None, default_markers()).unwrap());
    assert!(!newest_turn_is_human_ask(Some(&[]), default_markers()).unwrap());
}

fn assert_reminder_only_only_messages() {
    let messages = [text(
        Role::User,
        "<system-reminder>only plumbing</system-reminder>",
    )];
    assert_eq!(newest_turn_ask(&messages, default_markers()).unwrap(), None);
    assert!(!newest_turn_is_human_ask(Some(&messages), default_markers()).unwrap());
}

fn assert_newest_turn_all_system() {
    let messages = [text(Role::System, "system only")];
    assert!(!newest_turn_is_human_ask(Some(&messages), default_markers()).unwrap());
}

fn assert_codex_prefix_needs_separator() {
    assert!(!is_codex_user_agent("codex"));
    assert!(!is_codex_user_agent("xcodex-rs/1.0"));
    assert!(!is_codex_user_agent("codexrs/1.0"));
}

fn assert_codex_prefix_cases() {
    assert!(is_codex_user_agent("codex-cli/1.0"));
    assert!(is_codex_user_agent("Codex_CLI/2.0"));
    assert!(is_codex_user_agent("CODEX cli"));
    assert!(is_codex_user_agent("codex/1.0"));
}

fn assert_plan_tools_not_list() {
    let body = body(serde_json::json!({"tools": "exit_plan_mode"}));
    let result = matched_plan_mode_sentinel(Some(&body), None, &[], default_markers()).unwrap();
    assert_eq!(result, None);
}

fn assert_plan_function_name_mismatch_skips_top_level() {
    let body = body(serde_json::json!({
        "tools": [{"type": "function", "function": {"name": "other"}, "name": PLAN_MODE_TOOL_NAME}]
    }));
    let result = matched_plan_mode_sentinel(Some(&body), None, &[], default_markers()).unwrap();
    assert_eq!(result, None);
}

fn assert_plan_sentinel_at_newest_ask_only() {
    let messages = [
        text(Role::User, "older ask"),
        text(Role::Assistant, "reply"),
        text(Role::User, TAIL_SENTINEL),
    ];
    assert_plan_pattern(&messages, TAIL_SENTINEL);
}

fn assert_plan_empty_pattern_matches_empty_text() {
    let messages = [text(Role::User, "   ")];
    let patterns = vec![String::new()];
    let result =
        matched_plan_mode_sentinel(None, Some(&messages), &patterns, default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(""));
}

fn assert_plan_empty_texts_skip_empty_pattern() {
    let messages = [chat(Role::User, None), chat(Role::User, None)];
    let patterns = vec![String::new()];
    let result =
        matched_plan_mode_sentinel(None, Some(&messages), &patterns, default_markers()).unwrap();
    assert_eq!(result, None);
}

fn assert_plan_empty_body_system_matches_empty_pattern() {
    let body = body(serde_json::json!({"system": ""}));
    let patterns = vec![String::new()];
    let result =
        matched_plan_mode_sentinel(Some(&body), None, &patterns, default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(""));
}

fn assert_plan_body_messages_non_list_falls_back() {
    let body = body(serde_json::json!({"messages": "not a list"}));
    let resolved = [text(Role::User, TAIL_SENTINEL)];
    let result =
        matched_plan_mode_sentinel(Some(&body), Some(&resolved), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(TAIL_SENTINEL));
}

fn assert_plan_newest_ask_before_assistant() {
    let messages = [
        text(Role::User, TAIL_SENTINEL),
        text(Role::Assistant, "reply"),
    ];
    assert_plan_pattern(&messages, TAIL_SENTINEL);
}

fn assert_plan_function_type_without_object() {
    let body = body(serde_json::json!({
        "tools": [{"type": "function", "name": PLAN_MODE_TOOL_NAME}]
    }));
    let result = matched_plan_mode_sentinel(Some(&body), None, &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(PLAN_MODE_TOOL_NAME));
}

fn assert_encrypted_input_not_list() {
    assert_encrypted_absent(body(serde_json::json!("not a list")), Vec::new());
    assert_encrypted_input(None, Vec::new(), Ok(None));
}

fn assert_encrypted_item_not_object() {
    assert_encrypted_absent(
        body(serde_json::json!([7, {"type": "agent_message"}])),
        Vec::new(),
    );
}

fn assert_encrypted_content_not_list() {
    assert_encrypted_absent_with_ask(body(serde_json::json!([
        {"type": "agent_message", "content": "plain"}
    ])));
}

fn assert_encrypted_part_not_object() {
    assert_encrypted_absent_with_ask(body(serde_json::json!([
        {
            "type": "agent_message",
            "content": [7, {"type": "encrypted_content", "encrypted_content": "blob"}]
        }
    ])));
}

fn assert_encrypted_resolved_without_ask() {
    assert_encrypted_absent(
        Json::List(vec![encrypted_item()]),
        vec![vec![body(
            serde_json::json!({"role": "assistant", "content": "reply"}),
        )]],
    );
}

fn assert_encrypted_non_agent_selected() {
    assert_encrypted_absent_with_ask(body(serde_json::json!([
        {
            "type": "message",
            "content": [{"type": "encrypted_content", "encrypted_content": "blob"}]
        }
    ])));
}

fn assert_encrypted_empty_string_absent() {
    assert_encrypted_absent_with_ask(body(serde_json::json!([
        {
            "type": "agent_message",
            "content": [{"type": "encrypted_content", "encrypted_content": ""}]
        }
    ])));
}

fn assert_encrypted_non_string_errors() {
    assert_encrypted_input_error(
        body(serde_json::json!([
            {
                "type": "agent_message",
                "content": [{"type": "encrypted_content", "encrypted_content": 7}]
            }
        ])),
        ClassifierTextError::EncryptedContentNotString,
    );
}

#[rstest]
#[case::empty_message_list(assert_empty_message_list)]
#[case::blank_text_is_not_ask(assert_blank_text_is_not_ask)]
#[case::empty_markers_strip_nothing(assert_empty_markers_strip_nothing)]
#[case::marker_at_both_ends(assert_marker_at_both_ends)]
#[case::overlapping_marker_pairs(assert_overlapping_marker_pairs)]
#[case::text_part_not_string_names_index(assert_text_part_not_string_names_index)]
#[case::image_url_not_string_names_field(assert_image_url_not_string_names_field)]
#[case::json_text_part_null_names_index(assert_json_text_part_null_names_index)]
#[case::last_ask_index_with_system_tail(assert_last_ask_index_with_system_tail)]
#[case::last_ask_index_not_zero(assert_last_ask_index_not_zero)]
#[case::strip_block_at_end(assert_strip_block_at_end)]
#[case::newest_turn_no_messages(assert_newest_turn_no_messages)]
#[case::reminder_only_only_messages(assert_reminder_only_only_messages)]
#[case::newest_turn_all_system(assert_newest_turn_all_system)]
#[case::codex_prefix_needs_separator(assert_codex_prefix_needs_separator)]
#[case::codex_prefix_cases(assert_codex_prefix_cases)]
#[case::plan_tools_not_list(assert_plan_tools_not_list)]
#[case::plan_function_name_mismatch_skips_top_level(
    assert_plan_function_name_mismatch_skips_top_level
)]
#[case::plan_sentinel_at_newest_ask_only(assert_plan_sentinel_at_newest_ask_only)]
#[case::plan_empty_pattern_matches_empty_text(assert_plan_empty_pattern_matches_empty_text)]
#[case::plan_empty_texts_skip_empty_pattern(assert_plan_empty_texts_skip_empty_pattern)]
#[case::plan_empty_body_system_matches_empty_pattern(
    assert_plan_empty_body_system_matches_empty_pattern
)]
#[case::plan_body_messages_non_list_falls_back(assert_plan_body_messages_non_list_falls_back)]
#[case::plan_newest_ask_before_assistant(assert_plan_newest_ask_before_assistant)]
#[case::plan_function_type_without_object(assert_plan_function_type_without_object)]
#[case::encrypted_input_not_list(assert_encrypted_input_not_list)]
#[case::encrypted_item_not_object(assert_encrypted_item_not_object)]
#[case::encrypted_content_not_list(assert_encrypted_content_not_list)]
#[case::encrypted_part_not_object(assert_encrypted_part_not_object)]
#[case::encrypted_resolved_without_ask(assert_encrypted_resolved_without_ask)]
#[case::encrypted_non_agent_selected(assert_encrypted_non_agent_selected)]
#[case::encrypted_empty_string_absent(assert_encrypted_empty_string_absent)]
#[case::encrypted_non_string_errors(assert_encrypted_non_string_errors)]
fn classifier_message_text_boundaries(#[case] assertion: fn()) {
    assertion();
}
