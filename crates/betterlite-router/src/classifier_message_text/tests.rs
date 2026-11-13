#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use indexmap::IndexMap;
use rstest::rstest;

use crate::classifier_message_text::asks::{
    ExtractedAsk, conversation_is_continuing, current_ask_and_system_prompt,
    human_asks_newest_first, is_reminder_only_turn, json_human_asks_newest_first,
    last_human_ask_index, newest_turn_ask, newest_turn_is_human_ask,
};
use crate::classifier_message_text::encrypted::encrypted_classifier_task;
use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::image::inline_image_part;
use crate::classifier_message_text::markers::{
    CODEX_REMINDER_MARKERS, DEFAULT_REMINDER_MARKERS, MarkerPair, is_codex_user_agent,
    reminder_markers_for_request,
};
use crate::classifier_message_text::plan::{PLAN_MODE_TOOL_NAME, matched_plan_mode_sentinel};
use crate::classifier_message_text::text::{
    human_text, json_message_text, message_text, strip_reminder_blocks,
};

const SYSTEM_SENTINEL: &str = "You are currently running in \"Plan\" mode.";
const TAIL_SENTINEL: &str = "Plan mode is active";
const STILL_ACTIVE: &str = "Plan mode still active";

fn chat(role: Role, content: Option<MessageContent>) -> ChatMessage {
    ChatMessage {
        role,
        content,
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

fn text(role: Role, value: &str) -> ChatMessage {
    chat(role, Some(MessageContent::Text(value.to_owned())))
}

fn parts(role: Role, parts: Vec<ContentPart>) -> ChatMessage {
    chat(role, Some(MessageContent::Parts(parts)))
}

fn kind_part<T: serde::Serialize>(kind: &str, key: &str, value: T) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields: IndexMap::from([(key.to_owned(), serde_json::to_value(value).unwrap())]),
    }
}

fn text_part(value: &str) -> ContentPart {
    kind_part("text", "text", value)
}

fn body<T: serde::Serialize>(value: T) -> Json {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn default_markers() -> &'static [MarkerPair<'static>] {
    DEFAULT_REMINDER_MARKERS
}

fn codex_markers() -> &'static [MarkerPair<'static>] {
    CODEX_REMINDER_MARKERS
}

fn one_pair<'marker>(open: &'marker str, close: &'marker str) -> Vec<MarkerPair<'marker>> {
    vec![MarkerPair { open, close }]
}

fn ask_texts(asks: &[crate::classifier_message_text::asks::ExtractedAsk]) -> Vec<&str> {
    asks.iter().map(|ask| ask.text.as_str()).collect()
}

fn resolver(
    resolved: Vec<Vec<Json>>,
) -> impl FnMut(&Json) -> Result<Vec<Json>, ClassifierTextError> {
    let mut remaining = resolved.into_iter();
    move |_item| Ok(remaining.next().unwrap_or_default())
}

#[derive(Debug, Clone, Copy)]
enum Behavior {
    MessageTextJoinsTextParts,
    MessageTextKeepsPlainString,
    MessageTextDropsToolResults,
    StripDefaultReminder,
    StripCodexEnvelope,
    StripNestedLeavesOuterTail,
    StripUnclosedStays,
    StripCaseInsensitive,
    StripAdjacentBlocksJoin,
    HumanTextStripsBeforeMatching,
    NewestAskSkipsToolTurns,
    NewestAskSkipsReminderTurns,
    CurrentAskAndSystemPrompt,
    SystemPromptNotStripped,
    ConversationContinuingWithAssistant,
    ConversationContinuingWithoutMessages,
    ReminderOnlyTurn,
    NewestTurnIsHumanAsk,
    NewestTurnPlumbingIsNotAsk,
    MarkersDefault,
    MarkersCodexUserAgent,
    MarkersConfiguredWins,
    PlanToolShapeFunction,
    PlanToolShapeAnthropic,
    PlanSystemScope,
    PlanLeadingSystemMessage,
    PlanTailSentinel,
    PlanExtraPattern,
    PlanBodyMessagesWin,
    PlanStaleUserQuoteIsIgnored,
    PlanMarkerChoiceMovesStaleness,
    InlineImageDataUrl,
    InlineImageRemoteRejected,
    InlineImageAnthropicBase64,
    InlineImageInputImage,
    EncryptedTaskKeptContent,
    EncryptedTaskSkippedForMessages,
    EncryptedTaskSkippedForYoungerAsk,
    EncryptedTaskSkippedWithoutEncryptedPart,
}

#[rstest]
#[case::message_text_joins_text_parts(Behavior::MessageTextJoinsTextParts)]
#[case::message_text_keeps_plain_string(Behavior::MessageTextKeepsPlainString)]
#[case::message_text_drops_tool_results(Behavior::MessageTextDropsToolResults)]
#[case::strip_default_reminder(Behavior::StripDefaultReminder)]
#[case::strip_codex_envelope(Behavior::StripCodexEnvelope)]
#[case::strip_nested_leaves_outer_tail(Behavior::StripNestedLeavesOuterTail)]
#[case::strip_unclosed_stays(Behavior::StripUnclosedStays)]
#[case::strip_case_insensitive(Behavior::StripCaseInsensitive)]
#[case::strip_adjacent_blocks_join(Behavior::StripAdjacentBlocksJoin)]
#[case::human_text_strips_before_matching(Behavior::HumanTextStripsBeforeMatching)]
#[case::newest_ask_skips_tool_turns(Behavior::NewestAskSkipsToolTurns)]
#[case::newest_ask_skips_reminder_turns(Behavior::NewestAskSkipsReminderTurns)]
#[case::current_ask_and_system_prompt(Behavior::CurrentAskAndSystemPrompt)]
#[case::system_prompt_not_stripped(Behavior::SystemPromptNotStripped)]
#[case::conversation_continuing_with_assistant(Behavior::ConversationContinuingWithAssistant)]
#[case::conversation_continuing_without_messages(Behavior::ConversationContinuingWithoutMessages)]
#[case::reminder_only_turn(Behavior::ReminderOnlyTurn)]
#[case::newest_turn_is_human_ask(Behavior::NewestTurnIsHumanAsk)]
#[case::newest_turn_plumbing_is_not_ask(Behavior::NewestTurnPlumbingIsNotAsk)]
#[case::markers_default(Behavior::MarkersDefault)]
#[case::markers_codex_user_agent(Behavior::MarkersCodexUserAgent)]
#[case::markers_configured_wins(Behavior::MarkersConfiguredWins)]
#[case::plan_tool_shape_function(Behavior::PlanToolShapeFunction)]
#[case::plan_tool_shape_anthropic(Behavior::PlanToolShapeAnthropic)]
#[case::plan_system_scope(Behavior::PlanSystemScope)]
#[case::plan_leading_system_message(Behavior::PlanLeadingSystemMessage)]
#[case::plan_tail_sentinel(Behavior::PlanTailSentinel)]
#[case::plan_extra_pattern(Behavior::PlanExtraPattern)]
#[case::plan_body_messages_win(Behavior::PlanBodyMessagesWin)]
#[case::plan_stale_user_quote_is_ignored(Behavior::PlanStaleUserQuoteIsIgnored)]
#[case::plan_marker_choice_moves_staleness(Behavior::PlanMarkerChoiceMovesStaleness)]
#[case::inline_image_data_url(Behavior::InlineImageDataUrl)]
#[case::inline_image_remote_rejected(Behavior::InlineImageRemoteRejected)]
#[case::inline_image_anthropic_base64(Behavior::InlineImageAnthropicBase64)]
#[case::inline_image_input_image(Behavior::InlineImageInputImage)]
#[case::encrypted_task_kept_content(Behavior::EncryptedTaskKeptContent)]
#[case::encrypted_task_skipped_for_messages(Behavior::EncryptedTaskSkippedForMessages)]
#[case::encrypted_task_skipped_for_younger_ask(Behavior::EncryptedTaskSkippedForYoungerAsk)]
#[case::encrypted_task_skipped_without_encrypted_part(
    Behavior::EncryptedTaskSkippedWithoutEncryptedPart
)]
fn classifier_message_text_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::MessageTextJoinsTextParts => assert_message_text_joins_text_parts(),
        Behavior::MessageTextKeepsPlainString => assert_message_text_keeps_plain_string(),
        Behavior::MessageTextDropsToolResults => assert_message_text_drops_tool_results(),
        Behavior::StripDefaultReminder => assert_strip_default_reminder(),
        Behavior::StripCodexEnvelope => assert_strip_codex_envelope(),
        Behavior::StripNestedLeavesOuterTail => assert_strip_nested_leaves_outer_tail(),
        Behavior::StripUnclosedStays => assert_strip_unclosed_stays(),
        Behavior::StripCaseInsensitive => assert_strip_case_insensitive(),
        Behavior::StripAdjacentBlocksJoin => assert_strip_adjacent_blocks_join(),
        Behavior::HumanTextStripsBeforeMatching => assert_human_text_strips(),
        Behavior::NewestAskSkipsToolTurns => assert_newest_ask_skips_tool_turns(),
        Behavior::NewestAskSkipsReminderTurns => assert_newest_ask_skips_reminder_turns(),
        Behavior::CurrentAskAndSystemPrompt => assert_current_ask_and_system_prompt(),
        Behavior::SystemPromptNotStripped => assert_system_prompt_not_stripped(),
        Behavior::ConversationContinuingWithAssistant => {
            assert_conversation_continuing_with_assistant()
        }
        Behavior::ConversationContinuingWithoutMessages => {
            assert_conversation_continuing_without_messages()
        }
        Behavior::ReminderOnlyTurn => assert_reminder_only_turn(),
        Behavior::NewestTurnIsHumanAsk => assert_newest_turn_is_human_ask(),
        Behavior::NewestTurnPlumbingIsNotAsk => assert_newest_turn_plumbing_is_not_ask(),
        Behavior::MarkersDefault => assert_markers_default(),
        Behavior::MarkersCodexUserAgent => assert_markers_codex_user_agent(),
        Behavior::MarkersConfiguredWins => assert_markers_configured_wins(),
        Behavior::PlanToolShapeFunction => assert_plan_tool_shape_function(),
        Behavior::PlanToolShapeAnthropic => assert_plan_tool_shape_anthropic(),
        Behavior::PlanSystemScope => assert_plan_system_scope(),
        Behavior::PlanLeadingSystemMessage => assert_plan_leading_system_message(),
        Behavior::PlanTailSentinel => assert_plan_tail_sentinel(),
        Behavior::PlanExtraPattern => assert_plan_extra_pattern(),
        Behavior::PlanBodyMessagesWin => assert_plan_body_messages_win(),
        Behavior::PlanStaleUserQuoteIsIgnored => assert_plan_stale_user_quote_is_ignored(),
        Behavior::PlanMarkerChoiceMovesStaleness => assert_plan_marker_choice_moves_staleness(),
        Behavior::InlineImageDataUrl => assert_inline_image_data_url(),
        Behavior::InlineImageRemoteRejected => assert_inline_image_remote_rejected(),
        Behavior::InlineImageAnthropicBase64 => assert_inline_image_anthropic_base64(),
        Behavior::InlineImageInputImage => assert_inline_image_input_image(),
        Behavior::EncryptedTaskKeptContent => assert_encrypted_task_kept_content(),
        Behavior::EncryptedTaskSkippedForMessages => assert_encrypted_task_skipped_for_messages(),
        Behavior::EncryptedTaskSkippedForYoungerAsk => {
            assert_encrypted_task_skipped_for_younger_ask()
        }
        Behavior::EncryptedTaskSkippedWithoutEncryptedPart => {
            assert_encrypted_task_skipped_without_encrypted_part();
        }
    }
}

fn assert_message_text_joins_text_parts() {
    let message = parts(
        Role::User,
        vec![
            text_part("first"),
            kind_part("image_url", "image_url", serde_json::json!("data:x")),
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
            serde_json::json!("tool output"),
        )],
    );
    assert_eq!(message_text(message.content.as_ref()).unwrap(), "");
}

fn assert_strip_default_reminder() {
    assert_eq!(
        strip_reminder_blocks(
            "before <system-reminder>hidden</system-reminder> after",
            default_markers()
        ),
        "before after"
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
    assert_eq!(
        strip_reminder_blocks(
            "ask <environment_context>ctx</environment_context> now",
            default_markers()
        ),
        "ask <environment_context>ctx</environment_context> now"
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
    assert_eq!(
        strip_reminder_blocks(
            "<SYSTEM-REMINDER>upper</SYSTEM-REMINDER> keep",
            default_markers()
        ),
        "keep"
    );
}

fn assert_strip_adjacent_blocks_join() {
    assert_eq!(
        strip_reminder_blocks(
            "<system-reminder>a</system-reminder><system-reminder>b</system-reminder>",
            default_markers()
        ),
        ""
    );
}

fn assert_human_text_strips() {
    let content =
        MessageContent::Text("<system-reminder>noise</system-reminder> the real ask".to_owned());
    assert_eq!(
        human_text(Some(&content), default_markers()).unwrap(),
        "the real ask"
    );
}

fn assert_newest_ask_skips_tool_turns() {
    let messages = [
        text(Role::User, "first ask"),
        text(Role::Assistant, "calling the tool"),
        parts(
            Role::User,
            vec![kind_part(
                "tool_result",
                "content",
                serde_json::json!("done"),
            )],
        ),
    ];
    let asks = human_asks_newest_first(&messages, default_markers()).unwrap();
    assert_eq!(ask_texts(&asks), ["first ask"]);
    assert_eq!(asks.first().unwrap().message_index, 0);
}

fn assert_newest_ask_skips_reminder_turns() {
    let messages = [
        text(Role::User, "the ask"),
        text(Role::User, "<system-reminder>budget</system-reminder>"),
    ];
    let asks = human_asks_newest_first(&messages, default_markers()).unwrap();
    assert_eq!(ask_texts(&asks), ["the ask"]);
}

fn assert_current_ask_and_system_prompt() {
    let messages = [
        text(Role::System, "older system"),
        text(Role::User, "older ask"),
        text(Role::System, "newer system"),
        text(Role::Assistant, "reply"),
        text(Role::User, "newest ask"),
    ];
    let (ask, system) = current_ask_and_system_prompt(&messages, default_markers()).unwrap();
    assert_eq!(ask.unwrap().text, "newest ask");
    assert_eq!(system.unwrap().text, "newer system");
}

fn assert_system_prompt_not_stripped() {
    let messages = [
        text(
            Role::System,
            "<system-reminder>constraint</system-reminder>",
        ),
        text(Role::User, "ask"),
    ];
    let (_, system) = current_ask_and_system_prompt(&messages, default_markers()).unwrap();
    assert_eq!(
        system.unwrap().text,
        "<system-reminder>constraint</system-reminder>"
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
            vec![kind_part(
                "tool_result",
                "content",
                serde_json::json!("done"),
            )],
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
    let body = body(serde_json::json!({
        "tools": [{"type": "function", "function": {"name": PLAN_MODE_TOOL_NAME}}]
    }));
    let result = matched_plan_mode_sentinel(Some(&body), None, &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(PLAN_MODE_TOOL_NAME));
}

fn assert_plan_tool_shape_anthropic() {
    let body = body(serde_json::json!({
        "tools": [{"name": PLAN_MODE_TOOL_NAME, "input_schema": {}}]
    }));
    let result = matched_plan_mode_sentinel(Some(&body), None, &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(PLAN_MODE_TOOL_NAME));
}

fn assert_plan_system_scope() {
    let body = body(serde_json::json!({"system": SYSTEM_SENTINEL}));
    let result = matched_plan_mode_sentinel(Some(&body), None, &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(SYSTEM_SENTINEL));
}

fn assert_plan_leading_system_message() {
    let messages = [text(Role::System, SYSTEM_SENTINEL), text(Role::User, "ask")];
    let result = matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(SYSTEM_SENTINEL));
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
    let body = body(serde_json::json!({"messages": [{"role": "user", "content": STILL_ACTIVE}]}));
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
        serde_json::json!("data:image/png;base64,AAAA"),
    );
    let image = inline_image_part(&part).unwrap().unwrap();
    assert_eq!(image.url, "data:image/png;base64,AAAA");
    let object = kind_part(
        "image_url",
        "image_url",
        serde_json::json!({"url": "data:image/png;base64,BBBB"}),
    );
    let image = inline_image_part(&object).unwrap().unwrap();
    assert_eq!(image.url, "data:image/png;base64,BBBB");
}

fn assert_inline_image_remote_rejected() {
    let part = kind_part(
        "image_url",
        "image_url",
        serde_json::json!("https://example.test/a.png"),
    );
    assert_eq!(inline_image_part(&part).unwrap(), None);
    let missing = kind_part("image_url", "image_url", serde_json::json!({"url": ""}));
    assert_eq!(inline_image_part(&missing).unwrap(), None);
}

fn assert_inline_image_anthropic_base64() {
    let part = kind_part(
        "image",
        "source",
        serde_json::json!({"type": "base64", "media_type": "image/gif", "data": "CCCC"}),
    );
    let image = inline_image_part(&part).unwrap().unwrap();
    assert_eq!(image.url, "data:image/gif;base64,CCCC");
    let remote = kind_part(
        "image",
        "source",
        serde_json::json!({"type": "url", "url": "data:x"}),
    );
    assert_eq!(inline_image_part(&remote).unwrap().unwrap().url, "data:x");
}

fn assert_inline_image_input_image() {
    let part = kind_part(
        "input_image",
        "image_url",
        serde_json::json!("data:image/png;base64,DDDD"),
    );
    let image = inline_image_part(&part).unwrap().unwrap();
    assert_eq!(image.url, "data:image/png;base64,DDDD");
}

fn encrypted_item() -> Json {
    body(serde_json::json!({
        "type": "agent_message",
        "id": "item-1",
        "content": [
            {"type": "input_text", "text": "hello"},
            {"type": "encrypted_content", "encrypted_content": "blob"},
            {"type": "tool_result", "content": "dropped"}
        ]
    }))
}

fn resolved_ask() -> Vec<Json> {
    vec![body(
        serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hello blob"}]}),
    )]
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
        Some(&body(
            serde_json::json!({"type": "input_text", "text": "hello"})
        ))
    );
    assert_eq!(
        kept.get(1),
        Some(&body(serde_json::json!({
            "type": "encrypted_content",
            "encrypted_content": "blob"
        })))
    );
}

fn assert_encrypted_task_skipped_for_messages() {
    let input = Json::List(vec![encrypted_item()]);
    let task = encrypted_classifier_task::<ClassifierTextError, _>(
        Some(&input),
        true,
        resolver(vec![resolved_ask()]),
        default_markers(),
    )
    .unwrap();
    assert_eq!(task, None);
}

fn assert_encrypted_task_skipped_for_younger_ask() {
    let items = vec![
        encrypted_item(),
        body(serde_json::json!({"type": "message", "role": "user", "content": "later ask"})),
    ];
    let input = Json::List(items);
    let resolved = vec![
        resolved_ask(),
        vec![body(
            serde_json::json!({"role": "user", "content": "later ask"}),
        )],
    ];
    let task =
        encrypted_classifier_task(Some(&input), false, resolver(resolved), default_markers())
            .unwrap();
    assert_eq!(task, None);
}

fn assert_encrypted_task_skipped_without_encrypted_part() {
    let items = vec![body(serde_json::json!({
        "type": "agent_message",
        "content": [{"type": "input_text", "text": "hello"}]
    }))];
    let input = Json::List(items);
    let task = encrypted_classifier_task(
        Some(&input),
        false,
        resolver(vec![resolved_ask()]),
        default_markers(),
    )
    .unwrap();
    assert_eq!(task, None);
}

#[derive(Debug, Clone, Copy)]
enum Boundary {
    EmptyMessageList,
    BlankTextIsNotAsk,
    EmptyMarkersStripNothing,
    MarkerAtBothEnds,
    OverlappingMarkerPairs,
    TextPartNotStringNamesIndex,
    JsonTextPartNullNamesIndex,
    LastAskIndexWithSystemTail,
    NewestTurnNoMessages,
    ReminderOnlyOnlyMessages,
    NewestTurnAllSystem,
    CodexPrefixNeedsSeparator,
    CodexPrefixCases,
    PlanToolsNotList,
    PlanFunctionNameMismatchSkipsTopLevel,
    PlanSentinelAtNewestAskOnly,
    PlanEmptyPatternMatchesEmptyText,
    PlanBodyMessagesNonListFallsBack,
    EncryptedInputNotList,
    EncryptedItemNotObject,
    EncryptedContentNotList,
    EncryptedPartNotObject,
    EncryptedEmptyStringAbsent,
    EncryptedNonStringErrors,
}

#[rstest]
#[case::empty_message_list(Boundary::EmptyMessageList)]
#[case::blank_text_is_not_ask(Boundary::BlankTextIsNotAsk)]
#[case::empty_markers_strip_nothing(Boundary::EmptyMarkersStripNothing)]
#[case::marker_at_both_ends(Boundary::MarkerAtBothEnds)]
#[case::overlapping_marker_pairs(Boundary::OverlappingMarkerPairs)]
#[case::text_part_not_string_names_index(Boundary::TextPartNotStringNamesIndex)]
#[case::json_text_part_null_names_index(Boundary::JsonTextPartNullNamesIndex)]
#[case::last_ask_index_with_system_tail(Boundary::LastAskIndexWithSystemTail)]
#[case::newest_turn_no_messages(Boundary::NewestTurnNoMessages)]
#[case::reminder_only_only_messages(Boundary::ReminderOnlyOnlyMessages)]
#[case::newest_turn_all_system(Boundary::NewestTurnAllSystem)]
#[case::codex_prefix_needs_separator(Boundary::CodexPrefixNeedsSeparator)]
#[case::codex_prefix_cases(Boundary::CodexPrefixCases)]
#[case::plan_tools_not_list(Boundary::PlanToolsNotList)]
#[case::plan_function_name_mismatch_skips_top_level(
    Boundary::PlanFunctionNameMismatchSkipsTopLevel
)]
#[case::plan_sentinel_at_newest_ask_only(Boundary::PlanSentinelAtNewestAskOnly)]
#[case::plan_empty_pattern_matches_empty_text(Boundary::PlanEmptyPatternMatchesEmptyText)]
#[case::plan_body_messages_non_list_falls_back(Boundary::PlanBodyMessagesNonListFallsBack)]
#[case::encrypted_input_not_list(Boundary::EncryptedInputNotList)]
#[case::encrypted_item_not_object(Boundary::EncryptedItemNotObject)]
#[case::encrypted_content_not_list(Boundary::EncryptedContentNotList)]
#[case::encrypted_part_not_object(Boundary::EncryptedPartNotObject)]
#[case::encrypted_empty_string_absent(Boundary::EncryptedEmptyStringAbsent)]
#[case::encrypted_non_string_errors(Boundary::EncryptedNonStringErrors)]
fn classifier_message_text_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::EmptyMessageList => assert_empty_message_list(),
        Boundary::BlankTextIsNotAsk => assert_blank_text_is_not_ask(),
        Boundary::EmptyMarkersStripNothing => assert_empty_markers_strip_nothing(),
        Boundary::MarkerAtBothEnds => assert_marker_at_both_ends(),
        Boundary::OverlappingMarkerPairs => assert_overlapping_marker_pairs(),
        Boundary::TextPartNotStringNamesIndex => assert_text_part_not_string_names_index(),
        Boundary::JsonTextPartNullNamesIndex => assert_json_text_part_null_names_index(),
        Boundary::LastAskIndexWithSystemTail => assert_last_ask_index_with_system_tail(),
        Boundary::NewestTurnNoMessages => assert_newest_turn_no_messages(),
        Boundary::ReminderOnlyOnlyMessages => assert_reminder_only_only_messages(),
        Boundary::NewestTurnAllSystem => assert_newest_turn_all_system(),
        Boundary::CodexPrefixNeedsSeparator => assert_codex_prefix_needs_separator(),
        Boundary::CodexPrefixCases => assert_codex_prefix_cases(),
        Boundary::PlanToolsNotList => assert_plan_tools_not_list(),
        Boundary::PlanFunctionNameMismatchSkipsTopLevel => {
            assert_plan_function_name_mismatch_skips_top_level();
        }
        Boundary::PlanSentinelAtNewestAskOnly => assert_plan_sentinel_at_newest_ask_only(),
        Boundary::PlanEmptyPatternMatchesEmptyText => {
            assert_plan_empty_pattern_matches_empty_text()
        }
        Boundary::PlanBodyMessagesNonListFallsBack => {
            assert_plan_body_messages_non_list_falls_back();
        }
        Boundary::EncryptedInputNotList => assert_encrypted_input_not_list(),
        Boundary::EncryptedItemNotObject => assert_encrypted_item_not_object(),
        Boundary::EncryptedContentNotList => assert_encrypted_content_not_list(),
        Boundary::EncryptedPartNotObject => assert_encrypted_part_not_object(),
        Boundary::EncryptedEmptyStringAbsent => assert_encrypted_empty_string_absent(),
        Boundary::EncryptedNonStringErrors => assert_encrypted_non_string_errors(),
    }
}

fn assert_empty_message_list() {
    assert_eq!(
        human_asks_newest_first(&[], default_markers()).unwrap(),
        Vec::new()
    );
    assert_eq!(newest_turn_ask(&[], default_markers()).unwrap(), None);
    assert_eq!(last_human_ask_index(&[], default_markers()).unwrap(), None);
    assert_eq!(
        current_ask_and_system_prompt(&[], default_markers()).unwrap(),
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
    assert_eq!(
        strip_reminder_blocks(
            "<system-reminder>a</system-reminder>keep<system-reminder>b</system-reminder>",
            default_markers()
        ),
        "keep"
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
        "x 3</a> y"
    );
}

fn assert_text_part_not_string_names_index() {
    let part = kind_part("text", "text", serde_json::json!(7));
    let error =
        message_text(Some(&MessageContent::Parts(vec![text_part("ok"), part]))).unwrap_err();
    assert_eq!(error, ClassifierTextError::PartTextNotString { part: 1 });
}

fn assert_json_text_part_null_names_index() {
    let content = body(serde_json::json!([
        {"type": "text", "text": "fine"},
        {"type": "text", "text": null}
    ]));
    let error = json_message_text(Some(&content)).unwrap_err();
    assert_eq!(error, ClassifierTextError::PartTextNotString { part: 1 });
}

fn assert_last_ask_index_with_system_tail() {
    let messages = [
        text(Role::User, "ask"),
        text(Role::System, "trailing system"),
    ];
    assert_eq!(
        last_human_ask_index(&messages, default_markers()).unwrap(),
        Some(0)
    );
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
    let result = matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(TAIL_SENTINEL));
}

fn assert_plan_empty_pattern_matches_empty_text() {
    let messages = [text(Role::User, "   ")];
    let patterns = vec![String::new()];
    let result =
        matched_plan_mode_sentinel(None, Some(&messages), &patterns, default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(""));
}

fn assert_plan_body_messages_non_list_falls_back() {
    let body = body(serde_json::json!({"messages": "not a list"}));
    let resolved = [text(Role::User, TAIL_SENTINEL)];
    let result =
        matched_plan_mode_sentinel(Some(&body), Some(&resolved), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(TAIL_SENTINEL));
}

fn assert_encrypted_input_not_list() {
    let input = body(serde_json::json!("not a list"));
    let task =
        encrypted_classifier_task(Some(&input), false, resolver(Vec::new()), default_markers())
            .unwrap();
    assert_eq!(task, None);
    let task = encrypted_classifier_task::<ClassifierTextError, _>(
        None,
        false,
        resolver(Vec::new()),
        default_markers(),
    )
    .unwrap();
    assert_eq!(task, None);
}

fn assert_encrypted_item_not_object() {
    let input = body(serde_json::json!([7, {"type": "agent_message"}]));
    let task =
        encrypted_classifier_task(Some(&input), false, resolver(Vec::new()), default_markers())
            .unwrap();
    assert_eq!(task, None);
}

fn assert_encrypted_content_not_list() {
    let items = vec![body(serde_json::json!({
        "type": "agent_message",
        "content": "plain"
    }))];
    let input = Json::List(items);
    let task = encrypted_classifier_task(
        Some(&input),
        false,
        resolver(vec![resolved_ask()]),
        default_markers(),
    )
    .unwrap();
    assert_eq!(task, None);
}

fn assert_encrypted_part_not_object() {
    let items = vec![body(serde_json::json!({
        "type": "agent_message",
        "content": [7, {"type": "encrypted_content", "encrypted_content": "blob"}]
    }))];
    let input = Json::List(items);
    let task = encrypted_classifier_task(
        Some(&input),
        false,
        resolver(vec![resolved_ask()]),
        default_markers(),
    )
    .unwrap();
    assert_eq!(task, None);
}

fn assert_encrypted_empty_string_absent() {
    let items = vec![body(serde_json::json!({
        "type": "agent_message",
        "content": [{"type": "encrypted_content", "encrypted_content": ""}]
    }))];
    let input = Json::List(items);
    let task = encrypted_classifier_task(
        Some(&input),
        false,
        resolver(vec![resolved_ask()]),
        default_markers(),
    )
    .unwrap();
    assert_eq!(task, None);
}

fn assert_encrypted_non_string_errors() {
    let items = vec![body(serde_json::json!({
        "type": "agent_message",
        "content": [{"type": "encrypted_content", "encrypted_content": 7}]
    }))];
    let input = Json::List(items);
    let error = encrypted_classifier_task(
        Some(&input),
        false,
        resolver(vec![resolved_ask()]),
        default_markers(),
    )
    .unwrap_err();
    assert_eq!(error, ClassifierTextError::EncryptedContentNotString);
}

#[test]
fn json_human_asks_use_raw_message_roles() {
    let messages = vec![
        body(
            serde_json::json!({"role": "user", "content": "<system-reminder>x</system-reminder> ask"}),
        ),
        body(serde_json::json!({"role": "assistant", "content": "reply"})),
        body(serde_json::json!({"role": "user", "content": 7})),
    ];
    let asks = json_human_asks_newest_first(&messages, default_markers()).unwrap();
    assert_eq!(ask_texts(&asks), ["ask"]);
    assert_eq!(asks.first().unwrap().message_index, 0);
}
