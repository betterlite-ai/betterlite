use std::collections::BTreeMap;

use betterlite_prompts::message_content::Message;
use betterlite_prompts::system_placement::text_parts::CONVERTED_SYSTEM_NOTE;

use crate::anthropic_capability_catalog::thinking::DROP_DISABLED_THINKING_WARNING;
use crate::capabilities::speed::DROP_UNSUPPORTED_SPEED_WARNING;
use crate::params::output_config::DROP_OUTPUT_CONFIG_WARNING;
use crate::request::BillingHeader;
use crate::request::system::translate_system_message;

use super::fixture::{InliningFetcher, Outcome, Scenario, expect_absent, expect_field};

const ONE_USER: &str = r#"[{"role":"user","content":"u"}]"#;
const TOOL_USER: &str = r#"[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_1","content":"sunny"}]"#;
const DROP_THINKING_WARNING: &str = "Dropping 'thinking' param because the last assistant message with tool_calls has no thinking_blocks. The model won't use extended thinking for this turn.";

fn one_user(model: &str) -> Scenario {
    Scenario::new(model, ONE_USER)
}

fn beta_is(outcome: &Outcome, expected: &str) {
    assert_eq!(outcome.beta(), Some(expected));
}

fn refused(outcome: Outcome) -> (Option<u16>, String) {
    outcome.failed()
}

#[test]
fn case_0_leading_system_run() {
    leading_run_blocks();
    system_option_survives_empty();
    every_system_message_leaves();
}

fn leading_run_blocks() {
    let scenario = Scenario::new(
        "plain-model",
        r#"[{"role":"system","content":"lead"},{"role":"system","content":[{"type":"text","text":"second","cache_control":{"type":"ephemeral","ttl":"1h"}},{"type":"image_url","image_url":{"url":"x"}},{"type":"text","text":""}]},{"role":"user","content":"hi"}]"#,
    )
    .optional(r#"{"system":"configured"}"#);
    let outcome = scenario.run().ok();
    expect_field(
        &outcome,
        "system",
        r#"[{"type":"text","text":"lead"},{"type":"text","text":"second","cache_control":{"type":"ephemeral","ttl":"1h"}},{"type":"image_url","text":null}]"#,
    );
    expect_field(
        &outcome,
        "messages",
        r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#,
    );
}

// The public translation removes every system message it was given, not only the leading run it renders
fn every_system_message_leaves() {
    let source = r#"[{"role":"system","content":"lead"},{"role":"user","content":"u"},{"role":"system","content":[{"type":"text","text":"tail"}]}]"#;
    let mut messages: Vec<Message> = serde_json::from_str(source).expect("messages");
    let blocks = translate_system_message(&mut messages, false);
    assert_eq!(blocks.len(), 2);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
}

fn system_option_survives_empty() {
    let kept = one_user("plain-model")
        .optional(r#"{"system":"configured"}"#)
        .run()
        .ok();
    expect_field(&kept, "system", r#""configured""#);

    let empty = Scenario::new(
        "plain-model",
        r#"[{"role":"system","content":""},{"role":"user","content":"hi"}]"#,
    )
    .optional(r#"{"system":"configured"}"#)
    .run()
    .ok();
    expect_field(&empty, "system", r#""configured""#);
}

#[test]
fn case_1_mid_conversation_system() {
    flagged_keeps_the_system_role();
    mid_system_becomes_user_turn("plain-model");
    mid_system_becomes_user_turn("claude-sonnet-4.6-20260219");
    empty_history_needs_modify();
}

const MID_CONVERSATION: &str = r#"[{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"role":"system","content":"reminder"},{"role":"user","content":"u2"}]"#;

fn flagged_keeps_the_system_role() {
    let outcome = Scenario::new("mid-model", MID_CONVERSATION).run().ok();
    expect_absent(&outcome, "system");
    expect_field(
        &outcome,
        "messages",
        r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"a"}]},{"role":"user","content":[{"type":"text","text":"u2"}]},{"role":"system","content":[{"type":"text","text":"reminder"}]}]"#,
    );
}

// The candidate walk finds the flagged base entry for the dated alias, but LiteLLM's factory lookup does not
fn mid_system_becomes_user_turn(model: &str) {
    let outcome = Scenario::new(model, MID_CONVERSATION).run().ok();
    expect_absent(&outcome, "system");
    let note = serde_json::to_string(CONVERTED_SYSTEM_NOTE).expect("note");
    let expected = r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"a"}]},{"role":"user","content":[{"type":"text","text":__NOTE__},{"type":"text","text":"reminder"},{"type":"text","text":"u2"}]}]"#
        .replace("__NOTE__", &note);
    expect_field(&outcome, "messages", &expected);
}

fn empty_history_needs_modify() {
    let (status, message) = refused(Scenario::new("plain-model", "[]").run());
    assert_eq!(status, Some(400));
    assert!(message.ends_with("Received Messages=[]"), "{message}");

    let continued = Scenario::new("plain-model", "[]").modify().run().ok();
    expect_field(
        &continued,
        "messages",
        r#"[{"role":"user","content":[{"type":"text","text":"Please continue."}]}]"#,
    );
}

#[test]
fn case_2_missing_tool_definition() {
    dummy_tools_fill_missing();
    declared_tools_are_renamed();
    valid_names_keep_no_reverse_map();
    advisor_blocks_follow_tool();
    unreadable_tools_are_named();
}

fn unreadable_tools_are_named() {
    let (status, message) = refused(one_user("plain-model").optional(r#"{"tools":5}"#).run());
    assert_eq!(status, Some(400));
    assert_eq!(message, "tools must be a list");

    let (entry_status, entry_message) =
        refused(one_user("plain-model").optional(r#"{"tools":[5]}"#).run());
    assert_eq!(entry_status, Some(400));
    assert_eq!(entry_message, "tools entry 0 is not an object");
}

fn dummy_tools_fill_missing() {
    let injected = Scenario::new("plain-model", TOOL_USER).run().ok();
    expect_field(
        &injected,
        "tools",
        r#"[{"name":"dummy_tool","input_schema":{"type":"object","properties":{}},"type":"custom","description":"This is a dummy tool call"}]"#,
    );

    let declared_empty = Scenario::new("plain-model", TOOL_USER)
        .optional(r#"{"tools":[]}"#)
        .run()
        .ok();
    expect_field(&declared_empty, "tools", "[]");

    let declared_null = Scenario::new("plain-model", TOOL_USER)
        .optional(r#"{"tools":null}"#)
        .run()
        .ok();
    expect_field(&declared_null, "tools", "null");
}

fn declared_tools_are_renamed() {
    let scenario = Scenario::new(
        "plain-model",
        r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"my.tool","arguments":"{}"}}]}]"#,
    )
    .optional(
        r#"{"tools":[{"type":"custom","name":"my.tool","input_schema":{"type":"object","properties":{}},"description":"d"}],"tool_choice":{"type":"tool","name":"my.tool"}}"#,
    )
    .headers(r#"{"x-api-key":"k"}"#);
    let params = scenario.optional_params.clone();
    let headers = scenario.headers.clone();
    let outcome = scenario.run().ok();
    assert_eq!(scenario.optional_params, params, "the options changed");
    assert_eq!(scenario.headers, headers, "the headers changed");
    assert_eq!(outcome.beta(), None, "a plain custom tool adds no beta");
    expect_field(
        &outcome,
        "tools",
        r#"[{"type":"custom","name":"my_tool","input_schema":{"type":"object","properties":{}},"description":"d"}]"#,
    );
    expect_field(
        &outcome,
        "tool_choice",
        r#"{"type":"tool","name":"my_tool"}"#,
    );
    expect_field(
        &outcome,
        "messages",
        r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"my_tool","input":{}}]}]"#,
    );
    assert_eq!(
        outcome.reverse,
        BTreeMap::from([("my_tool".to_owned(), "my.tool".to_owned())])
    );
    expect_absent(&outcome, "_anthropic_tool_name_map");
}

fn valid_names_keep_no_reverse_map() {
    let valid = one_user("plain-model")
        .optional(
            r#"{"tools":[{"type":"custom","name":"plain_tool","input_schema":{"type":"object","properties":{}}}]}"#,
        )
        .run()
        .ok();
    assert!(valid.reverse.is_empty(), "{:?}", valid.reverse);
    expect_field(
        &valid,
        "tools",
        r#"[{"type":"custom","name":"plain_tool","input_schema":{"type":"object","properties":{}}}]"#,
    );
}

fn advisor_blocks_follow_tool() {
    let history = r#"[{"role":"user","content":"u"},{"role":"assistant","content":[{"type":"server_tool_use","id":"srvtoolu_1","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"srvtoolu_1","content":[]},{"type":"text","text":"answer"}]}]"#;
    let stripped = Scenario::new("plain-model", history).run().ok();
    expect_field(
        &stripped,
        "messages",
        r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"answer"}]}]"#,
    );

    let kept = Scenario::new("plain-model", history)
        .optional(r#"{"tools":[{"type":"advisor_20260301","name":"advisor"}]}"#)
        .run()
        .ok();
    expect_field(
        &kept,
        "messages",
        r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"server_tool_use","id":"srvtoolu_1","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"srvtoolu_1","content":[]},{"type":"text","text":"answer"}]}]"#,
    );
    assert!(
        kept.beta()
            .is_some_and(|value| value.contains("advisor-tool-2026-03-01")),
        "{:?}",
        kept.beta()
    );
}

#[test]
fn case_3_thinking_history_warning() {
    unpaired_thinking_is_dropped();
    thinking_survives_the_rest();
}

const THINKING: &str = r#"{"type":"enabled","budget_tokens":1024}"#;
const UNPAIRED_THINKING: &str = r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}]"#;

fn thinking_options() -> String {
    format!(r#"{{"thinking":{THINKING}}}"#)
}

fn unpaired_thinking_is_dropped() {
    let dropped = Scenario::new("plain-model", UNPAIRED_THINKING)
        .optional(&thinking_options())
        .modify()
        .run()
        .ok();
    expect_absent(&dropped, "thinking");
    assert_eq!(dropped.warnings, vec![DROP_THINKING_WARNING.to_owned()]);

    let kept = Scenario::new("plain-model", UNPAIRED_THINKING)
        .optional(&thinking_options())
        .run()
        .ok();
    expect_field(&kept, "thinking", THINKING);
    assert!(kept.warnings.is_empty(), "{:?}", kept.warnings);

    let null = Scenario::new("plain-model", UNPAIRED_THINKING)
        .optional(r#"{"thinking":null}"#)
        .modify()
        .run()
        .ok();
    expect_field(&null, "thinking", "null");
    assert!(null.warnings.is_empty(), "{:?}", null.warnings);
}

fn thinking_survives_the_rest() {
    let paired = r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"thinking_blocks":[{"type":"thinking","thinking":"t","signature":"sig"}],"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}]"#;
    let thought = Scenario::new("plain-model", paired)
        .optional(&thinking_options())
        .modify()
        .run()
        .ok();
    expect_field(&thought, "thinking", THINKING);
    assert!(thought.warnings.is_empty(), "{:?}", thought.warnings);

    let idle = Scenario::new(
        "plain-model",
        r#"[{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    )
    .optional(&thinking_options())
    .modify()
    .run()
    .ok();
    expect_field(&idle, "thinking", THINKING);
    assert!(idle.warnings.is_empty(), "{:?}", idle.warnings);
}

#[test]
fn case_4_billing_metadata_policy() {
    let stripped = Scenario::new("plain-model", BILLING_HISTORY)
        .strip_billing()
        .run()
        .ok();
    expect_field(&stripped, "system", STRIPPED_SYSTEM);
    let kept = Scenario::new("plain-model", BILLING_HISTORY).run().ok();
    expect_field(&kept, "system", KEPT_SYSTEM);
    assert_eq!(BillingHeader::from(true), BillingHeader::Strip);
    assert_eq!(BillingHeader::from(false), BillingHeader::Keep);
}

const BILLING_HISTORY: &str = r#"[{"role":"system","content":"x-anthropic-billing-header: abc"},{"role":"system","content":"kept"},{"role":"system","content":[{"type":"text","text":"x-anthropic-billing-header: x"},{"type":"text","text":" x-anthropic-billing-header: y"},{"type":"text","text":"X-anthropic-billing-header: z"},{"type":"text","text":"KEEP"}]},{"role":"user","content":"hi"}]"#;

const STRIPPED_SYSTEM: &str = r#"[{"type":"text","text":"kept"},{"type":"text","text":" x-anthropic-billing-header: y"},{"type":"text","text":"X-anthropic-billing-header: z"},{"type":"text","text":"KEEP"}]"#;
const KEPT_SYSTEM: &str = r#"[{"type":"text","text":"x-anthropic-billing-header: abc"},{"type":"text","text":"kept"},{"type":"text","text":"x-anthropic-billing-header: x"},{"type":"text","text":" x-anthropic-billing-header: y"},{"type":"text","text":"X-anthropic-billing-header: z"},{"type":"text","text":"KEEP"}]"#;

#[test]
fn case_5_user_id_metadata() {
    control_metadata_user_id();
    option_metadata_user_id();
    configured_metadata_default();
}

fn control_metadata_user_id() {
    let accepted = one_user("plain-model")
        .litellm(r#"{"metadata":{"user_id":"user-42"}}"#)
        .run()
        .ok();
    expect_field(&accepted, "metadata", r#"{"user_id":"user-42"}"#);

    let email = one_user("plain-model")
        .litellm(r#"{"metadata":{"user_id":"person@example.com"}}"#)
        .run()
        .ok();
    expect_absent(&email, "metadata");

    let (status, message) = refused(
        one_user("plain-model")
            .litellm(r#"{"metadata":{"user_id":5}}"#)
            .run(),
    );
    assert_eq!(status, Some(400));
    assert_eq!(message, "metadata.user_id must be a string");

    let null = one_user("plain-model")
        .litellm(r#"{"metadata":{"user_id":null}}"#)
        .run()
        .ok();
    expect_absent(&null, "metadata");
}

fn option_metadata_user_id() {
    let preexisting = one_user("plain-model")
        .optional(r#"{"metadata":{"user_id":"sent@example.com","team":"x"}}"#)
        .run()
        .ok();
    expect_field(
        &preexisting,
        "metadata",
        r#"{"user_id":"sent@example.com"}"#,
    );

    let other_keys = one_user("plain-model")
        .optional(r#"{"metadata":{"team":"x"}}"#)
        .run()
        .ok();
    expect_absent(&other_keys, "metadata");

    let null_id = one_user("plain-model")
        .optional(r#"{"metadata":{"user_id":null}}"#)
        .run()
        .ok();
    expect_absent(&null_id, "metadata");

    let not_an_object = one_user("plain-model")
        .optional(r#"{"metadata":"text"}"#)
        .run()
        .ok();
    expect_field(&not_an_object, "metadata", r#""text""#);

    let overridden = one_user("plain-model")
        .optional(r#"{"metadata":{"team":"x"}}"#)
        .litellm(r#"{"metadata":{"user_id":"user-42"}}"#)
        .run()
        .ok();
    expect_field(&overridden, "metadata", r#"{"user_id":"user-42"}"#);
}

fn configured_metadata_default() {
    let configured = one_user("plain-model")
        .defaults(r#"{"metadata":{"user_id":"cfg-user"}}"#)
        .run()
        .ok();
    expect_field(&configured, "metadata", r#"{"user_id":"cfg-user"}"#);

    let blocked = one_user("plain-model")
        .optional(r#"{"metadata":null}"#)
        .defaults(r#"{"metadata":{"user_id":"cfg-user"}}"#)
        .run()
        .ok();
    expect_field(&blocked, "metadata", "null");
}

#[test]
fn case_6_container_upload_tool() {
    upload_becomes_a_block();
    code_execution_tool_appended();
    declared_tools_stay_alone();
}

const UPLOAD_HISTORY: &str =
    r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"file-abc"}}]}]"#;
const UPLOAD_MESSAGES: &str =
    r#"[{"role":"user","content":[{"type":"container_upload","file_id":"file-abc"}]}]"#;
const WEATHER_TOOL: &str =
    r#"{"type":"custom","name":"weather","input_schema":{"type":"object","properties":{}}}"#;

fn upload_becomes_a_block() {
    let outcome = Scenario::new("plain-model", UPLOAD_HISTORY).run().ok();
    expect_field(&outcome, "messages", UPLOAD_MESSAGES);
    expect_absent(&outcome, "tools");
}

fn code_execution_tool_appended() {
    let outcome = Scenario::new("plain-model", UPLOAD_HISTORY)
        .optional(&format!(r#"{{"tools":[{WEATHER_TOOL}]}}"#))
        .run()
        .ok();
    expect_field(&outcome, "messages", UPLOAD_MESSAGES);
    let expected =
        format!(r#"[{WEATHER_TOOL},{{"name":"code_execution","type":"code_execution_20250522"}}]"#);
    expect_field(&outcome, "tools", &expected);
}

fn declared_tools_stay_alone() {
    let hosted = Scenario::new("plain-model", UPLOAD_HISTORY)
        .optional(r#"{"tools":[{"type":"code_execution_20250101","name":"code_execution"}]}"#)
        .run()
        .ok();
    expect_field(
        &hosted,
        "tools",
        r#"[{"type":"code_execution_20250101","name":"code_execution"}]"#,
    );

    let plain = one_user("plain-model")
        .optional(&format!(r#"{{"tools":[{WEATHER_TOOL}]}}"#))
        .run()
        .ok();
    expect_field(&plain, "tools", &format!("[{WEATHER_TOOL}]"));

    let listed = Scenario::new(
        "plain-model",
        r#"[{"role":"user","content":[{"type":"text","text":"u"}]}]"#,
    )
    .optional(&format!(r#"{{"tools":[{WEATHER_TOOL}]}}"#))
    .run()
    .ok();
    expect_field(&listed, "tools", &format!("[{WEATHER_TOOL}]"));
}

#[test]
fn case_7_optional_beta_headers() {
    vertex_requests_skip_betas();
    tool_and_format_betas();
    context_management_betas();
    compaction_betas();
}

fn vertex_requests_skip_betas() {
    let vertex = one_user("fast-model")
        .optional(
            r#"{"is_vertex_request":true,"speed":"fast","output_format":{"type":"json_schema"}}"#,
        )
        .run()
        .ok();
    assert_eq!(vertex.beta(), None);
    expect_absent(&vertex, "is_vertex_request");
}

fn tool_and_format_betas() {
    let speed = one_user("fast-model")
        .optional(r#"{"speed":"fast"}"#)
        .run()
        .ok();
    beta_is(&speed, "fast-mode-2026-02-01");

    let slow = one_user("fast-model")
        .optional(r#"{"speed":"slow"}"#)
        .run()
        .ok();
    assert_eq!(slow.beta(), None);

    let legacy_format = one_user("plain-model")
        .optional(r#"{"output_format":{"type":"json_schema"}}"#)
        .run()
        .ok();
    beta_is(&legacy_format, "structured-outputs-2025-11-13");

    let null_format = one_user("plain-model")
        .optional(r#"{"output_format":null}"#)
        .run()
        .ok();
    assert_eq!(null_format.beta(), None);

    let nested_null = one_user("plain-model")
        .optional(r#"{"output_config":{"format":null}}"#)
        .run()
        .ok();
    assert_eq!(nested_null.beta(), None);

    let fetched = one_user("plain-model")
        .optional(r#"{"tools":[{"type":"web_fetch_20250910","name":"web_fetch"}]}"#)
        .run()
        .ok();
    beta_is(&fetched, "web-fetch-2025-09-10");

    let remembered = one_user("plain-model")
        .optional(r#"{"tools":[{"type":"memory_20250818","name":"memory"}]}"#)
        .run()
        .ok();
    beta_is(&remembered, "context-management-2025-06-27");
}

fn context_management_betas() {
    let compact_edit = one_user("plain-model")
        .optional(r#"{"context_management":{"edits":[{"type":"compact_20260112"}]}}"#)
        .run()
        .ok();
    beta_is(&compact_edit, "compact-2026-01-12");

    let other_edit = one_user("plain-model")
        .optional(r#"{"context_management":{"edits":[{"type":"other"}]}}"#)
        .run()
        .ok();
    beta_is(&other_edit, "context-management-2025-06-27");

    let list_form = one_user("plain-model")
        .optional(r#"{"context_management":[{"type":"compact_20260112"},{"type":"other"}]}"#)
        .run()
        .ok();
    beta_is(
        &list_form,
        "compact-2026-01-12, context-management-2025-06-27",
    );

    let no_edits = one_user("plain-model")
        .optional(r#"{"context_management":{"other":1}}"#)
        .run()
        .ok();
    assert_eq!(no_edits.beta(), None);

    let (status, message) = refused(
        one_user("plain-model")
            .optional(r#"{"context_management":{"edits":"nope"}}"#)
            .run(),
    );
    assert_eq!(status, Some(400));
    assert_eq!(message, "context_management.edits must be a list");

    let (entry_status, entry_message) = refused(
        one_user("plain-model")
            .optional(r#"{"context_management":{"edits":[5]}}"#)
            .run(),
    );
    assert_eq!(entry_status, Some(400));
    assert_eq!(entry_message, "context_management edit 0 is not an object");
}

fn compaction_betas() {
    let compaction = one_user("plain-model")
        .optional(r#"{"compaction":{"strategy":"auto"}}"#)
        .run()
        .ok();
    beta_is(&compaction, "compact-2026-09-04");

    let other_provider = one_user("plain-model")
        .optional(r#"{"compaction":{"strategy":"auto"}}"#)
        .provider("vertex_ai")
        .run()
        .ok();
    assert_eq!(other_provider.beta(), None);

    let signed = Scenario::new(
        "plain-model",
        r#"[{"role":"user","content":"u"},{"role":"assistant","content":"answer","provider_specific_fields":{"compaction_blocks":[{"type":"compaction","content":"summary","signature":"sig"}]}}]"#,
    )
    .run()
    .ok();
    beta_is(&signed, "compact-2026-09-04");

    let unsigned = Scenario::new(
        "plain-model",
        r#"[{"role":"user","content":"u"},{"role":"assistant","content":"answer","provider_specific_fields":{"compaction_blocks":[{"type":"compaction","content":"summary","signature":""}]}}]"#,
    )
    .run()
    .ok();
    assert_eq!(unsigned.beta(), None);
}

#[test]
fn case_8_inline_media_only() {
    http_images_are_refused();
    other_media_never_downloads();
}

fn http_images_are_refused() {
    let history = r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"http://example.test/a.png"}}]}]"#;
    let outcome = Scenario::new("plain-model", history).run();
    let calls = outcome.fetch_calls;
    let (status, message) = refused(outcome);
    assert_eq!(calls, 1);
    assert_eq!(status, Some(400));
    assert!(
        message.contains("MAX_IMAGE_URL_DOWNLOAD_SIZE_MB=0"),
        "{message}"
    );
    assert!(message.contains("http://example.test/a.png"), "{message}");
}

fn other_media_never_downloads() {
    let https_image = Scenario::new(
        "plain-model",
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}]"#,
    )
    .run()
    .ok();
    assert_eq!(https_image.fetch_calls, 0);
    expect_field(
        &https_image,
        "messages",
        r#"[{"role":"user","content":[{"type":"image","source":{"type":"url","url":"https://example.test/a.png"}}]}]"#,
    );

    let data_image = Scenario::new(
        "plain-model",
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"data:image/png;base64,QUJD"}]}]"#,
    )
    .run()
    .ok();
    assert_eq!(data_image.fetch_calls, 0);
    expect_field(
        &data_image,
        "messages",
        r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]}]"#,
    );

    let document_url = Scenario::new(
        "plain-model",
        r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"http://example.test/doc.pdf"}}]}]"#,
    )
    .run()
    .ok();
    assert_eq!(document_url.fetch_calls, 0);
    expect_field(
        &document_url,
        "messages",
        r#"[{"role":"user","content":[{"type":"document","source":{"type":"url","url":"http://example.test/doc.pdf"}}]}]"#,
    );
}

// The inline stage answers a plain-HTTP image, so the write-back carries its bytes and translation never fetches
#[test]
fn inlined_bytes_replace_url() {
    let history = r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"http://example.test/a.png"}}]}]"#;
    let outcome = Scenario::new("plain-model", history)
        .run_with(&InliningFetcher)
        .ok();
    expect_field(
        &outcome,
        "messages",
        r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]}]"#,
    );
}

#[test]
fn case_9_beta_merge_casing_order() {
    casings_merge_into_one_header();
    additions_keep_the_call_order();
}

fn casings_merge_into_one_header() {
    let merged = one_user("fast-model")
        .optional(r#"{"speed":"fast"}"#)
        .headers(
            r#"{"X-API-Key":"k","content-type":"application/json","x-anthropic-beta-extra":"keep","Anthropic-Beta":"foo, bar","anthropic-BETA":" baz , , foo "}"#,
        )
        .run()
        .ok();
    beta_is(&merged, "foo, bar, baz, fast-mode-2026-02-01");
    let names: Vec<&str> = merged.headers.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "X-API-Key",
            "content-type",
            "x-anthropic-beta-extra",
            "anthropic-beta"
        ]
    );
    assert_eq!(
        merged.headers.get("x-anthropic-beta-extra"),
        Some(&"keep".to_owned())
    );
}

fn additions_keep_the_call_order() {
    let outcome = one_user("fast-model")
        .optional(
            r#"{"speed":"fast","compaction":{"strategy":"auto"},"output_format":{"type":"json_schema"},"context_management":{"edits":[{"type":"other"}]},"tools":[{"type":"web_fetch_20250910","name":"web_fetch"},{"type":"memory_20250818","name":"memory"},{"type":"advisor_20260301","name":"advisor"}]}"#,
        )
        .run()
        .ok();
    beta_is(
        &outcome,
        "compact-2026-09-04, web-fetch-2025-09-10, context-management-2025-06-27, structured-outputs-2025-11-13, fast-mode-2026-02-01, advisor-tool-2026-03-01",
    );
}

#[test]
fn case_10_assembly_option_cleanup() {
    max_tokens_defaults();
    internal_options_are_removed();
    top_k_sampling_gate();
    speed_follows_the_speed_gate();
    disabled_thinking_dropped();
    output_config_is_applied_last();
    format_survives_effort_drop();
    options_override_body_keys();
}

fn max_tokens_defaults() {
    let plain = one_user("plain-model").run().ok();
    expect_field(&plain, "max_tokens", "4096");

    let configured = one_user("plain-model")
        .defaults(r#"{"max_tokens":100}"#)
        .run()
        .ok();
    expect_field(&configured, "max_tokens", "100");

    let explicit = one_user("plain-model")
        .optional(r#"{"max_tokens":5}"#)
        .defaults(r#"{"max_tokens":100}"#)
        .run()
        .ok();
    expect_field(&explicit, "max_tokens", "5");
}

fn internal_options_are_removed() {
    let cleaned = one_user("plain-model")
        .optional(r#"{"is_vertex_request":false,"client_metadata":{"session":"s"}}"#)
        .run()
        .ok();
    expect_absent(&cleaned, "is_vertex_request");
    expect_absent(&cleaned, "client_metadata");
}

fn top_k_sampling_gate() {
    let sampled = one_user("plain-model")
        .optional(r#"{"top_k":5}"#)
        .run()
        .ok();
    expect_field(&sampled, "top_k", "5");

    let (status, message) = refused(one_user("no-sampling").optional(r#"{"top_k":5}"#).run());
    assert_eq!(status, Some(400));
    assert!(message.contains("does not support top_k=5"), "{message}");

    let global = one_user("no-sampling")
        .optional(r#"{"top_k":5}"#)
        .global_drop()
        .run()
        .ok();
    expect_absent(&global, "top_k");

    let dropped = one_user("no-sampling")
        .optional(r#"{"top_k":5}"#)
        .litellm(r#"{"drop_params":true}"#)
        .run()
        .ok();
    expect_absent(&dropped, "top_k");

    let null = one_user("no-sampling")
        .optional(r#"{"top_k":null}"#)
        .run()
        .ok();
    expect_absent(&null, "top_k");
}

fn speed_follows_the_speed_gate() {
    let (status, message) = refused(
        one_user("plain-model")
            .optional(r#"{"speed":"fast"}"#)
            .run(),
    );
    assert_eq!(status, Some(400));
    assert!(message.contains("does not support speed"), "{message}");

    let dropped = one_user("plain-model")
        .optional(r#"{"speed":"fast"}"#)
        .litellm(r#"{"drop_params":true}"#)
        .run()
        .ok();
    expect_absent(&dropped, "speed");
    assert_eq!(
        dropped.warnings,
        vec![DROP_UNSUPPORTED_SPEED_WARNING.replacen("%s", "plain-model", 1)]
    );

    let global = one_user("plain-model")
        .optional(r#"{"speed":"fast"}"#)
        .global_drop()
        .run()
        .ok();
    expect_absent(&global, "speed");

    let kept = one_user("fast-model")
        .optional(r#"{"speed":"fast"}"#)
        .run()
        .ok();
    expect_field(&kept, "speed", r#""fast""#);
}

fn disabled_thinking_dropped() {
    let disabled = one_user("always-on")
        .optional(r#"{"thinking":{"type":"disabled"}}"#)
        .run()
        .ok();
    expect_absent(&disabled, "thinking");
    assert_eq!(
        disabled.warnings,
        vec![DROP_DISABLED_THINKING_WARNING.replacen("%s", "always-on", 1)]
    );

    let kept = one_user("plain-model")
        .optional(r#"{"thinking":{"type":"disabled"}}"#)
        .run()
        .ok();
    expect_field(&kept, "thinking", r#"{"type":"disabled"}"#);
}

fn output_config_is_applied_last() {
    let (status, message) = refused(
        one_user("plain-model")
            .optional(r#"{"output_config":{"effort":"huge"}}"#)
            .run(),
    );
    assert_eq!(status, Some(400));
    assert!(
        message.contains("Invalid effort value: 'huge'"),
        "{message}"
    );

    let (xhigh_status, xhigh_message) = refused(
        one_user("plain-model")
            .optional(r#"{"output_config":{"effort":"xhigh"}}"#)
            .run(),
    );
    assert_eq!(xhigh_status, Some(400));
    assert!(
        xhigh_message
            .contains("effort='xhigh' is not supported by this model. Got model: plain-model"),
        "{xhigh_message}"
    );

    let permissive = one_user("plain-model")
        .optional(r#"{"output_config":{"effort":"high"}}"#)
        .run()
        .ok();
    expect_field(&permissive, "output_config", r#"{"effort":"high"}"#);

    let effort = one_user("effort-model")
        .optional(r#"{"output_config":{"effort":"xhigh"}}"#)
        .run()
        .ok();
    expect_field(&effort, "output_config", r#"{"effort":"xhigh"}"#);
}

fn format_survives_effort_drop() {
    let preserved = one_user("plain-model")
        .optional(
            r#"{"output_config":{"format":{"type":"json_schema","schema":{"type":"object"}},"effort":"high"}}"#,
        )
        .global_drop()
        .run()
        .ok();
    expect_field(
        &preserved,
        "output_config",
        r#"{"format":{"type":"json_schema","schema":{"type":"object"}}}"#,
    );
    assert_eq!(
        preserved.warnings,
        vec![DROP_OUTPUT_CONFIG_WARNING.replacen("%s", "plain-model", 1)]
    );
}

fn options_override_body_keys() {
    let overridden = one_user("plain-model")
        .optional(r#"{"model":"override-model","messages":[]}"#)
        .run()
        .ok();
    expect_field(&overridden, "model", r#""override-model""#);
    expect_field(&overridden, "messages", "[]");
}
