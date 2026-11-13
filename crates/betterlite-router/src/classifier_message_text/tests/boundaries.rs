use rstest::rstest;

use super::*;

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
    LastAskIndexNotZero,
    StripBlockAtEnd,
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
    PlanNewestAskBeforeAssistant,
    PlanFunctionTypeWithoutObject,
    EncryptedInputNotList,
    EncryptedItemNotObject,
    EncryptedContentNotList,
    EncryptedPartNotObject,
    EncryptedResolvedWithoutAsk,
    EncryptedNonAgentSelected,
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
#[case::last_ask_index_not_zero(Boundary::LastAskIndexNotZero)]
#[case::strip_block_at_end(Boundary::StripBlockAtEnd)]
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
#[case::plan_newest_ask_before_assistant(Boundary::PlanNewestAskBeforeAssistant)]
#[case::plan_function_type_without_object(Boundary::PlanFunctionTypeWithoutObject)]
#[case::encrypted_input_not_list(Boundary::EncryptedInputNotList)]
#[case::encrypted_item_not_object(Boundary::EncryptedItemNotObject)]
#[case::encrypted_content_not_list(Boundary::EncryptedContentNotList)]
#[case::encrypted_part_not_object(Boundary::EncryptedPartNotObject)]
#[case::encrypted_resolved_without_ask(Boundary::EncryptedResolvedWithoutAsk)]
#[case::encrypted_non_agent_selected(Boundary::EncryptedNonAgentSelected)]
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
        Boundary::LastAskIndexNotZero => assert_last_ask_index_not_zero(),
        Boundary::StripBlockAtEnd => assert_strip_block_at_end(),
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
        Boundary::PlanNewestAskBeforeAssistant => assert_plan_newest_ask_before_assistant(),
        Boundary::PlanFunctionTypeWithoutObject => assert_plan_function_type_without_object(),
        Boundary::EncryptedInputNotList => assert_encrypted_input_not_list(),
        Boundary::EncryptedItemNotObject => assert_encrypted_item_not_object(),
        Boundary::EncryptedContentNotList => assert_encrypted_content_not_list(),
        Boundary::EncryptedPartNotObject => assert_encrypted_part_not_object(),
        Boundary::EncryptedResolvedWithoutAsk => assert_encrypted_resolved_without_ask(),
        Boundary::EncryptedNonAgentSelected => assert_encrypted_non_agent_selected(),
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

fn assert_last_ask_index_not_zero() {
    let messages = [
        text(Role::User, "<system-reminder>plumbing</system-reminder>"),
        text(Role::User, "the ask"),
    ];
    assert_eq!(
        last_human_ask_index(&messages, default_markers()).unwrap(),
        Some(1)
    );
}

fn assert_strip_block_at_end() {
    assert_eq!(
        strip_reminder_blocks(
            "keep <system-reminder></system-reminder>",
            default_markers()
        ),
        "keep"
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

fn assert_plan_newest_ask_before_assistant() {
    let messages = [
        text(Role::User, TAIL_SENTINEL),
        text(Role::Assistant, "reply"),
    ];
    let result = matched_plan_mode_sentinel(None, Some(&messages), &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(TAIL_SENTINEL));
}

fn assert_plan_function_type_without_object() {
    let body = body(serde_json::json!({
        "tools": [{"type": "function", "name": PLAN_MODE_TOOL_NAME}]
    }));
    let result = matched_plan_mode_sentinel(Some(&body), None, &[], default_markers()).unwrap();
    assert_eq!(result.as_deref(), Some(PLAN_MODE_TOOL_NAME));
}

fn assert_encrypted_resolved_without_ask() {
    let items = vec![encrypted_item()];
    let input = Json::List(items);
    let resolved = vec![vec![body(
        serde_json::json!({"role": "assistant", "content": "reply"}),
    )]];
    let task =
        encrypted_classifier_task(Some(&input), false, resolver(resolved), default_markers())
            .unwrap();
    assert_eq!(task, None);
}

fn assert_encrypted_non_agent_selected() {
    let items = vec![body(serde_json::json!({
        "type": "message",
        "content": [{"type": "encrypted_content", "encrypted_content": "blob"}]
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
