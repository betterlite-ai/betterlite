use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, Role};
use rstest::rstest;

use super::{
    STILL_ACTIVE, SYSTEM_SENTINEL, TAIL_SENTINEL, ask_texts, assert_strip, codex_markers,
    default_markers, encrypted_item, encrypted_part, json_number, json_object, json_text,
    kind_part, last_ask_index, one_pair, parts, plan_body_result, resolved_ask, resolver,
    system_body, text, text_part, tools_body, typed_item,
};
use crate::classifier_message_text::asks::{
    ExtractedAsk, conversation_is_continuing, current_ask_and_system_prompt,
    human_asks_newest_first, is_reminder_only_turn, newest_turn_ask, newest_turn_is_human_ask,
};
use crate::classifier_message_text::encrypted::encrypted_classifier_task;
use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::image::inline_image_part;
use crate::classifier_message_text::markers::reminder_markers_for_request;
use crate::classifier_message_text::plan::{PLAN_MODE_TOOL_NAME, matched_plan_mode_sentinel};
use crate::classifier_message_text::text::{message_text, strip_reminder_blocks};

#[test]
fn json_human_asks_use_raw_message_roles() {
    let messages = vec![
        json_object(&[
            ("role", json_text("user")),
            (
                "content",
                json_text("<system-reminder>x</system-reminder> ask"),
            ),
        ]),
        json_object(&[
            ("role", json_text("assistant")),
            ("content", json_text("reply")),
        ]),
        json_object(&[("role", json_text("user")), ("content", json_number(7))]),
    ];
    let asks = human_asks_newest_first(&messages, default_markers()).unwrap();
    assert_eq!(ask_texts(&asks), ["ask"]);
    assert_eq!(asks.first().unwrap().message_index, 0);
}

fn assert_asks(messages: &[ChatMessage], expected: &[&str]) -> Vec<ExtractedAsk> {
    let asks = human_asks_newest_first(messages, default_markers()).unwrap();
    assert_eq!(ask_texts(&asks), expected);
    asks
}

fn assert_extracted(messages: &[ChatMessage], expected_ask: &str, expected_system: &str) {
    let (ask, system) = current_ask_and_system_prompt(messages, default_markers()).unwrap();
    assert_eq!(ask.map(|found| found.text).as_deref(), Some(expected_ask));
    assert_eq!(
        system.map(|found| found.text).as_deref(),
        Some(expected_system)
    );
}

fn assert_encrypted_skip(items: Vec<Json>, has_messages: bool, resolved: Vec<Vec<Json>>) {
    let input = Json::List(items);
    let task = encrypted_classifier_task::<ClassifierTextError, _>(
        Some(&input),
        has_messages,
        resolver(resolved),
        default_markers(),
    )
    .unwrap();
    assert_eq!(task, None);
}

fn assert_message_text_joins_text_parts() {
    let message = parts(
        Role::User,
        vec![
            text_part("first"),
            kind_part("image_url", "image_url", json_text("data:x")),
            text_part("second"),
        ],
    );
    assert_eq!(
        message_text(message.content.as_ref()).unwrap(),
        "first second"
    );
}

fn assert_message_text_keeps_plain_string() {
    let message = text(Role::User, "  padded  ");
    assert_eq!(
        message_text(message.content.as_ref()).unwrap(),
        "  padded  "
    );
}

fn assert_message_text_drops_tool_results() {
    let message = parts(
        Role::User,
        vec![kind_part(
            "tool_result",
            "content",
            json_text("tool output"),
        )],
    );
    assert_eq!(message_text(message.content.as_ref()).unwrap(), "");
}

fn assert_strip_default_reminder() {
    assert_strip(
        "before <system-reminder>hidden</system-reminder> after",
        "before after",
    );
}

fn assert_strip_codex_envelope() {
    assert_eq!(
        strip_reminder_blocks(
            "ask <environment_context>ctx</environment_context> now",
            codex_markers()
        ),
        "ask now"
    );
    assert_strip(
        "ask <environment_context>ctx</environment_context> now",
        "ask <environment_context>ctx</environment_context> now",
    );
}

fn assert_strip_nested_leaves_outer_tail() {
    let text = "<system-reminder>outer <system-reminder>inner</system-reminder> tail</system-reminder> before";
    assert_eq!(
        strip_reminder_blocks(text, default_markers()),
        "tail</system-reminder> before"
    );
}

fn assert_strip_unclosed_stays() {
    let text = "<system-reminder>never closed";
    assert_eq!(strip_reminder_blocks(text, default_markers()), text);
}

fn assert_strip_case_insensitive() {
    assert_strip("<SYSTEM-REMINDER>upper</SYSTEM-REMINDER> keep", "keep");
}

fn assert_strip_adjacent_blocks_join() {
    assert_strip(
        "<system-reminder>a</system-reminder><system-reminder>b</system-reminder>",
        "",
    );
}

fn assert_human_text_strips_before_matching() {
    let messages = [text(
        Role::User,
        "<system-reminder>noise</system-reminder> the real ask",
    )];
    assert_asks(&messages, &["the real ask"]);
}

fn assert_newest_ask_skips_tool_turns() {
    let messages = [
        text(Role::User, "first ask"),
        text(Role::Assistant, "calling the tool"),
        parts(
            Role::User,
            vec![kind_part("tool_result", "content", json_text("done"))],
        ),
    ];
    let asks = assert_asks(&messages, &["first ask"]);
    assert_eq!(asks.first().unwrap().message_index, 0);
}

fn assert_newest_ask_skips_reminder_turns() {
    let messages = [
        text(Role::User, "the ask"),
        text(Role::User, "<system-reminder>budget</system-reminder>"),
    ];
    assert_asks(&messages, &["the ask"]);
}

fn assert_newest_turn_ask() {
    let messages = [
        text(Role::User, "older"),
        text(Role::Assistant, "reply"),
        text(Role::User, "newest"),
    ];
    let ask = newest_turn_ask(&messages, default_markers())
        .unwrap()
        .unwrap();
    assert_eq!(ask.text, "newest");
    assert_eq!(ask.message_index, 2);
}

fn assert_reminder_only_text_parts() {
    let message = parts(
        Role::User,
        vec![text_part("<system-reminder>noise</system-reminder>")],
    );
    assert!(is_reminder_only_turn(&message, default_markers()).unwrap());
}

fn assert_newest_turn_with_trailing_system() {
    let messages = [text(Role::User, "ask"), text(Role::System, "trailing")];
    assert!(newest_turn_is_human_ask(Some(&messages), default_markers()).unwrap());
}

fn assert_current_ask_and_system_prompt() {
    let messages = [
        text(Role::System, "older system"),
        text(Role::User, "older ask"),
        text(Role::System, "newer system"),
        text(Role::Assistant, "reply"),
        text(Role::User, "newest ask"),
    ];
    assert_extracted(&messages, "newest ask", "newer system");
}

fn assert_current_ask_with_codex_markers() {
    let messages = [text(
        Role::User,
        "the question <environment_context>ctx</environment_context>",
    )];
    let (ask, _) = current_ask_and_system_prompt(&messages, codex_markers()).unwrap();
    assert_eq!(ask.unwrap().text, "the question");
    let (unstripped, _) = current_ask_and_system_prompt(&messages, default_markers()).unwrap();
    assert_eq!(
        unstripped.unwrap().text,
        "the question <environment_context>ctx</environment_context>"
    );
}

fn assert_ask_functions_use_codex_markers() {
    let envelope = text(Role::User, "<environment_context>ctx</environment_context>");
    assert!(is_reminder_only_turn(&envelope, codex_markers()).unwrap());
    assert!(!is_reminder_only_turn(&envelope, default_markers()).unwrap());

    let newest_messages = [
        text(Role::Assistant, "reply"),
        text(Role::User, "<environment_context>ctx</environment_context>"),
    ];
    assert_eq!(
        newest_turn_ask(&newest_messages, codex_markers()).unwrap(),
        None
    );
    assert_eq!(
        newest_turn_ask(&newest_messages, default_markers())
            .unwrap()
            .unwrap()
            .text,
        "<environment_context>ctx</environment_context>"
    );
}

fn assert_ask_lists_use_codex_markers() {
    let messages = [
        text(Role::User, "the question"),
        text(Role::Assistant, "reply"),
        text(Role::User, "<environment_context>ctx</environment_context>"),
    ];
    assert_eq!(last_ask_index(&messages, codex_markers()), Some(0));
    assert_eq!(last_ask_index(&messages, default_markers()), Some(2));
    let codex_asks = human_asks_newest_first(&messages, codex_markers()).unwrap();
    assert_eq!(ask_texts(&codex_asks), ["the question"]);
    let default_asks = human_asks_newest_first(&messages, default_markers()).unwrap();
    assert_eq!(
        ask_texts(&default_asks),
        [
            "<environment_context>ctx</environment_context>",
            "the question"
        ]
    );
    assert!(!newest_turn_is_human_ask(Some(&messages), codex_markers()).unwrap());
    assert!(newest_turn_is_human_ask(Some(&messages), default_markers()).unwrap());
}

fn assert_system_prompt_not_stripped() {
    let messages = [
        text(
            Role::System,
            "<system-reminder>constraint</system-reminder>",
        ),
        text(Role::User, "ask"),
    ];
    assert_extracted(
        &messages,
        "ask",
        "<system-reminder>constraint</system-reminder>",
    );
}

fn assert_conversation_continuing_with_assistant() {
    let messages = [text(Role::User, "ask"), text(Role::Assistant, "reply")];
    assert!(conversation_is_continuing(Some(&messages)));
    let first_turn = [text(Role::User, "ask")];
    assert!(!conversation_is_continuing(Some(&first_turn)));
}

fn assert_conversation_continuing_without_messages() {
    assert!(conversation_is_continuing(None));
    assert!(conversation_is_continuing(Some(&[])));
}

fn assert_reminder_only_turn() {
    let reminder = text(Role::User, "<system-reminder>noise</system-reminder>");
    assert!(is_reminder_only_turn(&reminder, default_markers()).unwrap());
    let hybrid = text(Role::User, "<system-reminder>noise</system-reminder> ask");
    assert!(!is_reminder_only_turn(&hybrid, default_markers()).unwrap());
    let blank = text(Role::User, "   ");
    assert!(!is_reminder_only_turn(&blank, default_markers()).unwrap());
    let assistant = text(Role::Assistant, "<system-reminder>noise</system-reminder>");
    assert!(!is_reminder_only_turn(&assistant, default_markers()).unwrap());
}

fn assert_newest_turn_is_human_ask() {
    let messages = [text(Role::User, "ask")];
    assert!(newest_turn_is_human_ask(Some(&messages), default_markers()).unwrap());
}

fn assert_newest_turn_plumbing_is_not_ask() {
    let tool_turn = [
        text(Role::User, "ask"),
        parts(
            Role::User,
            vec![kind_part("tool_result", "content", json_text("done"))],
        ),
    ];
    assert!(!newest_turn_is_human_ask(Some(&tool_turn), default_markers()).unwrap());
    let assistant_turn = [text(Role::User, "ask"), text(Role::Assistant, "reply")];
    assert!(!newest_turn_is_human_ask(Some(&assistant_turn), default_markers()).unwrap());
    let reminder_tail = [
        text(Role::User, "ask"),
        text(Role::User, "<system-reminder>noise</system-reminder>"),
    ];
    assert!(newest_turn_is_human_ask(Some(&reminder_tail), default_markers()).unwrap());
}

fn assert_markers_default() {
    let selected = reminder_markers_for_request(None, false);
    assert_eq!(selected, default_markers());
}

fn assert_markers_codex_user_agent() {
    let selected = reminder_markers_for_request(None, true);
    assert_eq!(selected, codex_markers());
    assert_eq!(selected.len(), 6);
}

fn assert_markers_configured_wins() {
    let configured = one_pair("<ask>", "</ask>");
    let selected = reminder_markers_for_request(Some(&configured), true);
    assert_eq!(selected, configured.as_slice());
}

fn assert_plan_tool_shape_function() {
    let function = json_object(&[("name", json_text(PLAN_MODE_TOOL_NAME))]);
    let tool = json_object(&[("type", json_text("function")), ("function", function)]);
    let result = plan_body_result(&tools_body(vec![tool]));
    assert_eq!(result.as_deref(), Some(PLAN_MODE_TOOL_NAME));
}

fn assert_plan_tool_shape_anthropic() {
    let tool = json_object(&[
        ("name", json_text(PLAN_MODE_TOOL_NAME)),
        ("input_schema", json_object(&[])),
    ]);
    assert_eq!(
        plan_body_result(&tools_body(vec![tool])).as_deref(),
        Some(PLAN_MODE_TOOL_NAME)
    );
}

fn assert_plan_system_scope() {
    let body = system_body(json_text(SYSTEM_SENTINEL));
    assert_eq!(plan_body_result(&body).as_deref(), Some(SYSTEM_SENTINEL));
}

fn assert_plan_anthropic_system_array() {
    let part = json_object(&[
        ("type", json_text("text")),
        ("text", json_text(SYSTEM_SENTINEL)),
    ]);
    let body = system_body(Json::List(vec![part]));
    assert_eq!(plan_body_result(&body).as_deref(), Some(SYSTEM_SENTINEL));
}

fn assert_plan_system_array_skips_non_text() {
    let part = json_object(&[
        ("type", json_text("image_url")),
        ("text", json_text(SYSTEM_SENTINEL)),
    ]);
    let body = system_body(Json::List(vec![part]));
    assert_eq!(plan_body_result(&body), None);
}

fn assert_plan_leading_system_message() {
    let messages = [text(Role::System, SYSTEM_SENTINEL), text(Role::User, "ask")];
    let result = matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(SYSTEM_SENTINEL));
}

fn assert_plan_second_leading_system_message() {
    let messages = [
        text(Role::System, "opening"),
        text(Role::System, SYSTEM_SENTINEL),
        text(Role::User, "ask"),
    ];
    let result = matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(SYSTEM_SENTINEL));
}

fn assert_plan_later_system_message_ignored() {
    let messages = [
        text(Role::System, "opening"),
        text(Role::User, "ask"),
        text(Role::System, TAIL_SENTINEL),
        text(Role::User, "follow-up"),
    ];
    let result = matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(result, None);
}

fn assert_plan_tail_sentinel() {
    let messages = [text(Role::User, TAIL_SENTINEL)];
    let result = matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(TAIL_SENTINEL));
}

fn assert_plan_extra_pattern() {
    let messages = [text(Role::User, "an operator pattern")];
    let patterns = vec!["an operator pattern".to_owned()];
    let result =
        matched_plan_mode_sentinel(None, Some(&messages), &patterns, default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some("an operator pattern"));
}

fn assert_plan_body_messages_win() {
    let body = json_object(&[(
        "messages",
        Json::List(vec![json_object(&[
            ("role", json_text("user")),
            ("content", json_text(STILL_ACTIVE)),
        ])]),
    )]);
    let resolved = [text(Role::User, "plain ask")];
    let result =
        matched_plan_mode_sentinel(Some(&body), Some(&resolved), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(STILL_ACTIVE));
}

fn assert_plan_stale_user_quote_is_ignored() {
    let messages = [
        text(Role::User, SYSTEM_SENTINEL),
        text(Role::Assistant, "understood"),
        text(Role::User, "summarize the file"),
    ];
    let result = matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(result, None);
}

fn assert_plan_marker_choice_moves_staleness() {
    let messages = [
        text(Role::User, TAIL_SENTINEL),
        text(Role::User, "<environment_context>ctx</environment_context>"),
    ];
    let default_result =
        matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(default_result, None);
    let codex_result =
        matched_plan_mode_sentinel(None, Some(&messages), &[], codex_markers()).unwrap();
    assert_eq!(codex_result.as_deref(), Some(TAIL_SENTINEL));
}

fn assert_inline_image_data_url() {
    let part = kind_part(
        "image_url",
        "image_url",
        json_text("data:image/png;base64,AAAA"),
    );
    let image = inline_image_part(&part).unwrap().unwrap();
    assert_eq!(image.url, "data:image/png;base64,AAAA");
    let object = kind_part(
        "image_url",
        "image_url",
        json_object(&[("url", json_text("data:image/png;base64,BBBB"))]),
    );
    let second = inline_image_part(&object).unwrap().unwrap();
    assert_eq!(second.url, "data:image/png;base64,BBBB");
}

fn assert_inline_image_remote_rejected() {
    let part = kind_part(
        "image_url",
        "image_url",
        json_text("https://example.test/a.png"),
    );
    assert_eq!(inline_image_part(&part).unwrap(), None);
    let missing = kind_part(
        "image_url",
        "image_url",
        json_object(&[("url", json_text(""))]),
    );
    assert_eq!(inline_image_part(&missing).unwrap(), None);
}

fn assert_inline_image_anthropic_base64() {
    let part = kind_part(
        "image",
        "source",
        json_object(&[
            ("type", json_text("base64")),
            ("media_type", json_text("image/gif")),
            ("data", json_text("CCCC")),
        ]),
    );
    let image = inline_image_part(&part).unwrap().unwrap();
    assert_eq!(image.url, "data:image/gif;base64,CCCC");
    let remote = kind_part(
        "image",
        "source",
        json_object(&[("type", json_text("url")), ("url", json_text("data:x"))]),
    );
    assert_eq!(inline_image_part(&remote).unwrap().unwrap().url, "data:x");
}

fn assert_inline_image_input_image() {
    let part = kind_part(
        "input_image",
        "image_url",
        json_text("data:image/png;base64,DDDD"),
    );
    let image = inline_image_part(&part).unwrap().unwrap();
    assert_eq!(image.url, "data:image/png;base64,DDDD");
}

fn assert_encrypted_task_kept_content() {
    let items = vec![encrypted_item()];
    let input = Json::List(items);
    let task = encrypted_classifier_task(
        Some(&input),
        false,
        resolver(vec![resolved_ask()]),
        default_markers(),
    )
    .unwrap()
    .unwrap();
    let Json::Object(fields) = &task else {
        panic!("the task stays an object");
    };
    assert_eq!(fields.get("id"), Some(&Json::Text("item-1".to_owned())));
    let Some(Json::List(kept)) = fields.get("content") else {
        panic!("the task keeps a content list");
    };
    assert_eq!(kept.len(), 2);
    assert_eq!(
        kept.first(),
        Some(&json_object(&[
            ("type", json_text("input_text")),
            ("text", json_text("hello")),
        ]))
    );
    assert_eq!(kept.get(1), Some(&encrypted_part(json_text("blob"))));
}

fn assert_encrypted_task_skipped_for_messages() {
    assert_encrypted_skip(vec![encrypted_item()], true, vec![resolved_ask()]);
}

fn assert_encrypted_task_skipped_for_younger_ask() {
    let later_ask = json_object(&[
        ("role", json_text("user")),
        ("content", json_text("later ask")),
    ]);
    let items = vec![
        encrypted_item(),
        json_object(&[
            ("type", json_text("message")),
            ("role", json_text("user")),
            ("content", json_text("later ask")),
        ]),
    ];
    let resolved = vec![resolved_ask(), vec![later_ask]];
    assert_encrypted_skip(items, false, resolved);
}

fn assert_encrypted_task_skipped_without_encrypted_part() {
    let items = vec![typed_item(
        "agent_message",
        vec![json_object(&[
            ("type", json_text("input_text")),
            ("text", json_text("hello")),
        ])],
    )];
    assert_encrypted_skip(items, false, vec![resolved_ask()]);
}

#[rstest]
#[case::message_text_joins_text_parts(assert_message_text_joins_text_parts)]
#[case::message_text_keeps_plain_string(assert_message_text_keeps_plain_string)]
#[case::message_text_drops_tool_results(assert_message_text_drops_tool_results)]
#[case::strip_default_reminder(assert_strip_default_reminder)]
#[case::strip_codex_envelope(assert_strip_codex_envelope)]
#[case::strip_nested_leaves_outer_tail(assert_strip_nested_leaves_outer_tail)]
#[case::strip_unclosed_stays(assert_strip_unclosed_stays)]
#[case::strip_case_insensitive(assert_strip_case_insensitive)]
#[case::strip_adjacent_blocks_join(assert_strip_adjacent_blocks_join)]
#[case::human_text_strips_before_matching(assert_human_text_strips_before_matching)]
#[case::newest_ask_skips_tool_turns(assert_newest_ask_skips_tool_turns)]
#[case::newest_ask_skips_reminder_turns(assert_newest_ask_skips_reminder_turns)]
#[case::newest_turn_ask(assert_newest_turn_ask)]
#[case::reminder_only_text_parts(assert_reminder_only_text_parts)]
#[case::newest_turn_with_trailing_system(assert_newest_turn_with_trailing_system)]
#[case::current_ask_and_system_prompt(assert_current_ask_and_system_prompt)]
#[case::current_ask_with_codex_markers(assert_current_ask_with_codex_markers)]
#[case::ask_functions_use_codex_markers(assert_ask_functions_use_codex_markers)]
#[case::ask_lists_use_codex_markers(assert_ask_lists_use_codex_markers)]
#[case::system_prompt_not_stripped(assert_system_prompt_not_stripped)]
#[case::conversation_continuing_with_assistant(assert_conversation_continuing_with_assistant)]
#[case::conversation_continuing_without_messages(assert_conversation_continuing_without_messages)]
#[case::reminder_only_turn(assert_reminder_only_turn)]
#[case::newest_turn_is_human_ask(assert_newest_turn_is_human_ask)]
#[case::newest_turn_plumbing_is_not_ask(assert_newest_turn_plumbing_is_not_ask)]
#[case::markers_default(assert_markers_default)]
#[case::markers_codex_user_agent(assert_markers_codex_user_agent)]
#[case::markers_configured_wins(assert_markers_configured_wins)]
#[case::plan_tool_shape_function(assert_plan_tool_shape_function)]
#[case::plan_tool_shape_anthropic(assert_plan_tool_shape_anthropic)]
#[case::plan_system_scope(assert_plan_system_scope)]
#[case::plan_anthropic_system_array(assert_plan_anthropic_system_array)]
#[case::plan_system_array_skips_non_text(assert_plan_system_array_skips_non_text)]
#[case::plan_leading_system_message(assert_plan_leading_system_message)]
#[case::plan_second_leading_system_message(assert_plan_second_leading_system_message)]
#[case::plan_later_system_message_ignored(assert_plan_later_system_message_ignored)]
#[case::plan_tail_sentinel(assert_plan_tail_sentinel)]
#[case::plan_extra_pattern(assert_plan_extra_pattern)]
#[case::plan_body_messages_win(assert_plan_body_messages_win)]
#[case::plan_stale_user_quote_is_ignored(assert_plan_stale_user_quote_is_ignored)]
#[case::plan_marker_choice_moves_staleness(assert_plan_marker_choice_moves_staleness)]
#[case::inline_image_data_url(assert_inline_image_data_url)]
#[case::inline_image_remote_rejected(assert_inline_image_remote_rejected)]
#[case::inline_image_anthropic_base64(assert_inline_image_anthropic_base64)]
#[case::inline_image_input_image(assert_inline_image_input_image)]
#[case::encrypted_task_kept_content(assert_encrypted_task_kept_content)]
#[case::encrypted_task_skipped_for_messages(assert_encrypted_task_skipped_for_messages)]
#[case::encrypted_task_skipped_for_younger_ask(assert_encrypted_task_skipped_for_younger_ask)]
#[case::encrypted_task_skipped_without_encrypted_part(
    assert_encrypted_task_skipped_without_encrypted_part
)]
fn classifier_message_text_behaviors(#[case] assertion: fn()) {
    assertion();
}
