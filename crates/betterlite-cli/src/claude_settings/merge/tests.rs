#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;
use secrecy::SecretString;

use super::{MergeOptions, merge_claude_settings, with_status_line};
use crate::claude_settings::choices::{ModelChoice, StaticToken};
use crate::claude_settings::{
    ANTHROPIC_API_KEY_KEY, ANTHROPIC_AUTH_TOKEN_KEY, ANTHROPIC_BASE_URL_KEY, ANTHROPIC_MODEL_KEY,
    API_KEY_HELPER_KEY, BASE_URL_PATH, CREDENTIAL_PATHS, ENABLE_TOOL_SEARCH_KEY,
    ENABLE_TOOL_SEARCH_VALUE, ENV_KEY, GATEWAY_DISCOVERY_KEY, GATEWAY_DISCOVERY_VALUE, MODEL_KEY,
    MODEL_PATHS, OWNED_PATHS, STATUS_LINE_KEY, SettingsDocument, TIER_MODEL_ENV_KEYS,
};

const BASE_URL: &str = "https://proxy.example.com";
const TOKEN: &str = "sk-claude-token";
const COMMAND: &str = "'/usr/local/bin/lite' statusline";

fn settings(text: &str) -> SettingsDocument {
    serde_json::from_str::<SettingsDocument>(text).unwrap()
}

fn static_token() -> StaticToken {
    StaticToken {
        token: SecretString::from(TOKEN),
    }
}

fn options<'input>(
    base_url: &'input str,
    credential: &'input StaticToken,
    model: &'input ModelChoice,
) -> MergeOptions<'input> {
    MergeOptions {
        base_url,
        credential,
        model,
        tier_model: None,
        status_line: None,
    }
}

fn merged_with(original: &str, base_url: &str) -> SettingsDocument {
    let credential = static_token();
    let model = ModelChoice::KeepModel;
    merge_claude_settings(&settings(original), &options(base_url, &credential, &model))
}

fn merged(original: &str) -> SettingsDocument {
    merged_with(original, BASE_URL)
}

fn env_value<'document>(
    document: &'document SettingsDocument,
    key: &str,
) -> Option<&'document JsonValue> {
    document.get(ENV_KEY)?.as_object()?.get(key)
}

fn env_text<'document>(document: &'document SettingsDocument, key: &str) -> Option<&'document str> {
    env_value(document, key)?.as_str()
}

fn command_of(document: &SettingsDocument) -> Option<&str> {
    document
        .get(STATUS_LINE_KEY)?
        .as_object()?
        .get("command")?
        .as_str()
}

#[test]
fn merge_injects_the_base_url_and_the_token() {
    let document = merged(
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://old.example.com","ANTHROPIC_AUTH_TOKEN":"sk-old-token"}}"#,
    );
    assert_eq!(env_text(&document, ANTHROPIC_BASE_URL_KEY), Some(BASE_URL));
    assert_eq!(env_text(&document, ANTHROPIC_AUTH_TOKEN_KEY), Some(TOKEN));
    assert_eq!(document.get(API_KEY_HELPER_KEY), None);
}

#[rstest]
#[case::bare("https://proxy.example.com", "https://proxy.example.com")]
#[case::one_slash("https://proxy.example.com/", "https://proxy.example.com")]
#[case::many_slashes("https://proxy.example.com///", "https://proxy.example.com")]
#[case::path("https://proxy.example.com/v1/", "https://proxy.example.com/v1")]
fn merge_strips_trailing_slashes_from_the_base_url(#[case] base_url: &str, #[case] expected: &str) {
    let document = merged_with("{}", base_url);
    assert_eq!(env_text(&document, ANTHROPIC_BASE_URL_KEY), Some(expected));
}

#[test]
fn merge_keeps_other_env_entries() {
    let document = merged(r#"{"env":{"FOO":"bar","ANTHROPIC_MODEL":"user-model"}}"#);
    assert_eq!(env_text(&document, "FOO"), Some("bar"));
    assert_eq!(env_text(&document, ANTHROPIC_MODEL_KEY), Some("user-model"));
}

#[test]
fn merge_drops_the_api_key_helper_and_the_stray_api_key() {
    let document = merged(
        r#"{"apiKeyHelper":"/usr/local/bin/helper","env":{"ANTHROPIC_API_KEY":"sk-stray"}}"#,
    );
    assert_eq!(document.get(API_KEY_HELPER_KEY), None);
    assert_eq!(env_text(&document, ANTHROPIC_API_KEY_KEY), None);
    assert_eq!(env_text(&document, ANTHROPIC_AUTH_TOKEN_KEY), Some(TOKEN));
}

#[test]
fn merge_keeps_other_top_level_keys() {
    let document = merged(r#"{"model":"user-model","permissions":{"allow":["Bash"]}}"#);
    assert_eq!(
        document.get(MODEL_KEY),
        Some(&JsonValue::String("user-model".to_owned()))
    );
    assert!(document.contains_key("permissions"));
}

#[test]
fn merge_does_not_pin_a_model_for_keep_model() {
    let document = merged("{}");
    assert_eq!(document.get(MODEL_KEY), None);
    assert_eq!(env_text(&document, ANTHROPIC_MODEL_KEY), None);
}

#[test]
fn merge_sets_the_enable_defaults_when_missing() {
    let document = merged("{}");
    assert_eq!(
        env_text(&document, ENABLE_TOOL_SEARCH_KEY),
        Some(ENABLE_TOOL_SEARCH_VALUE)
    );
    assert_eq!(
        env_text(&document, GATEWAY_DISCOVERY_KEY),
        Some(GATEWAY_DISCOVERY_VALUE)
    );
    assert_eq!(ENABLE_TOOL_SEARCH_VALUE, "true");
    assert_eq!(GATEWAY_DISCOVERY_VALUE, "1");
}

#[test]
fn merge_keeps_existing_enable_values() {
    let original = format!(
        r#"{{"env":{{"{ENABLE_TOOL_SEARCH_KEY}":"false","{GATEWAY_DISCOVERY_KEY}":"0"}}}}"#
    );
    let document = merged(&original);
    assert_eq!(env_text(&document, ENABLE_TOOL_SEARCH_KEY), Some("false"));
    assert_eq!(env_text(&document, GATEWAY_DISCOVERY_KEY), Some("0"));
}

#[test]
fn merge_pins_the_start_pin_model() {
    let credential = static_token();
    let model = ModelChoice::StartOn("claude-sonnet-4".to_owned());
    let document = merge_claude_settings(
        &settings(r#"{"model":"user-model","env":{"ANTHROPIC_MODEL":"user-model"}}"#),
        &options(BASE_URL, &credential, &model),
    );
    assert_eq!(
        document.get(MODEL_KEY),
        Some(&JsonValue::String("claude-sonnet-4".to_owned()))
    );
    assert_eq!(
        env_text(&document, ANTHROPIC_MODEL_KEY),
        Some("claude-sonnet-4")
    );
}

#[rstest]
#[case::keep_model(ModelChoice::KeepModel)]
#[case::unpin_model(ModelChoice::UnpinModel)]
fn merge_leaves_the_model_alone_for_keep_and_unpin(#[case] model: ModelChoice) {
    let credential = static_token();
    let document = merge_claude_settings(
        &settings(r#"{"model":"user-model","env":{"ANTHROPIC_MODEL":"user-env-model"}}"#),
        &options(BASE_URL, &credential, &model),
    );
    assert_eq!(
        document.get(MODEL_KEY),
        Some(&JsonValue::String("user-model".to_owned()))
    );
    assert_eq!(
        env_text(&document, ANTHROPIC_MODEL_KEY),
        Some("user-env-model")
    );
}

#[test]
fn merge_points_every_tier_key_at_the_tier_model() {
    let credential = static_token();
    let model = ModelChoice::KeepModel;
    let document = merge_claude_settings(
        &settings(r#"{"env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"user-opus"}}"#),
        &MergeOptions {
            base_url: BASE_URL,
            credential: &credential,
            model: &model,
            tier_model: Some("autorouter"),
            status_line: None,
        },
    );
    assert_eq!(
        env_text(&document, "ANTHROPIC_DEFAULT_SONNET_MODEL"),
        Some("autorouter")
    );
    assert_eq!(
        env_text(&document, "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        Some("autorouter")
    );
    assert_eq!(
        env_text(&document, "ANTHROPIC_DEFAULT_OPUS_MODEL"),
        Some("autorouter")
    );
    assert_eq!(
        env_text(&document, "ANTHROPIC_DEFAULT_FABLE_MODEL"),
        Some("autorouter")
    );
}

#[test]
fn merge_leaves_the_tier_keys_alone_without_a_tier_model() {
    let document = merged(r#"{"env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"user-opus"}}"#);
    assert_eq!(
        env_text(&document, "ANTHROPIC_DEFAULT_OPUS_MODEL"),
        Some("user-opus")
    );
}

#[rstest]
#[case::text(r#"{"env":"oops"}"#)]
#[case::null(r#"{"env":null}"#)]
fn merge_replaces_a_non_object_env(#[case] original: &str) {
    let document = merged(original);
    assert_eq!(env_text(&document, ANTHROPIC_BASE_URL_KEY), Some(BASE_URL));
    assert_eq!(env_text(&document, ANTHROPIC_AUTH_TOKEN_KEY), Some(TOKEN));
}

#[test]
fn merge_leaves_the_status_line_alone_without_a_command() {
    let document = merged(r#"{"statusLine":{"type":"command","command":"/usr/bin/mine"}}"#);
    assert_eq!(command_of(&document), Some("/usr/bin/mine"));
}

#[test]
fn merge_does_not_add_a_status_line_without_a_command() {
    let document = merged("{}");
    assert_eq!(document.get(STATUS_LINE_KEY), None);
}

#[test]
fn merge_keeps_a_user_status_line_even_with_a_command() {
    let credential = static_token();
    let model = ModelChoice::KeepModel;
    let document = merge_claude_settings(
        &settings(r#"{"statusLine":{"type":"command","command":"/usr/bin/mine"}}"#),
        &MergeOptions {
            base_url: BASE_URL,
            credential: &credential,
            model: &model,
            tier_model: None,
            status_line: Some(COMMAND),
        },
    );
    assert_eq!(command_of(&document), Some("/usr/bin/mine"));
}

#[test]
fn merge_sets_our_status_line_when_a_command_is_given() {
    let credential = static_token();
    let model = ModelChoice::KeepModel;
    let document = merge_claude_settings(
        &settings("{}"),
        &MergeOptions {
            base_url: BASE_URL,
            credential: &credential,
            model: &model,
            tier_model: None,
            status_line: Some(COMMAND),
        },
    );
    assert_eq!(command_of(&document), Some(COMMAND));
}

#[test]
fn with_status_line_adds_an_entry_when_absent() {
    let document = with_status_line(&SettingsDocument::new(), COMMAND);
    let entry = document
        .get(STATUS_LINE_KEY)
        .and_then(JsonValue::as_object)
        .unwrap();
    assert_eq!(
        entry.get("type").and_then(JsonValue::as_str),
        Some("command")
    );
    assert_eq!(command_of(&document), Some(COMMAND));
}

#[test]
fn with_status_line_treats_a_null_entry_as_absent() {
    let document = with_status_line(&settings(r#"{"statusLine":null}"#), COMMAND);
    assert_eq!(command_of(&document), Some(COMMAND));
}

#[rstest]
#[case::older_binary("/opt/betterlite/bin/lite statusline")]
#[case::quoted_binary("'/opt/betterlite/bin/lite' statusline")]
#[case::relative_binary("target/debug/lite statusline")]
fn with_status_line_replaces_an_entry_that_runs_our_subcommand(#[case] existing: &str) {
    let original = settings(&format!(
        r#"{{"statusLine":{{"type":"command","command":"{existing}"}}}}"#
    ));
    let document = with_status_line(&original, COMMAND);
    assert_eq!(command_of(&document), Some(COMMAND));
}

#[rstest]
#[case::other_command(r#"{"statusLine":{"type":"command","command":"/usr/bin/mine"}}"#)]
#[case::entry_without_command(r#"{"statusLine":{"type":"command"}}"#)]
#[case::text_entry(r#"{"statusLine":"/usr/bin/mine"}"#)]
#[case::number_entry(r#"{"statusLine":7}"#)]
#[case::litellm_script(
    r#"{"statusLine":{"type":"command","command":"python3 /home/tester/.litellm/statusline.py"}}"#
)]
#[case::user_script(r#"{"statusLine":{"type":"command","command":"~/.claude/statusline.sh"}}"#)]
#[case::subcommand_runner(
    r#"{"statusLine":{"type":"command","command":"/usr/bin/statusline-runner"}}"#
)]
#[case::ccusage_via_bun(
    r#"{"statusLine":{"type":"command","command":"bun x ccusage statusline"}}"#
)]
#[case::ccusage_via_npx(
    r#"{"statusLine":{"type":"command","command":"npx -y ccusage statusline"}}"#
)]
fn with_status_line_leaves_another_entry_alone(#[case] original: &str) {
    let existing = settings(original);
    assert_eq!(with_status_line(&existing, COMMAND), existing);
}

#[test]
fn with_status_line_replaces_a_cmd_quoted_entry_for_the_same_binary() {
    let original = settings(
        r#"{"statusLine":{"type":"command","command":"\"C:\\old\\lite.exe\" \"statusline\""}}"#,
    );
    let command = "\"C:\\bin\\lite.exe\" \"statusline\"";
    let document = with_status_line(&original, command);
    assert_eq!(command_of(&document), Some(command));
}

#[test]
fn with_status_line_replaces_the_whole_entry() {
    let original =
        settings(r#"{"statusLine":{"type":"command","command":"lite statusline","pad":7}}"#);
    let document = with_status_line(&original, COMMAND);
    let entry = document
        .get(STATUS_LINE_KEY)
        .and_then(JsonValue::as_object)
        .unwrap();
    assert_eq!(entry.len(), 2);
    assert_eq!(entry.get("pad"), None);
}

#[test]
fn with_status_line_keeps_other_settings() {
    let original = settings(r#"{"model":"user-model","env":{"FOO":"bar"}}"#);
    let document = with_status_line(&original, COMMAND);
    assert_eq!(
        document.get(MODEL_KEY),
        Some(&JsonValue::String("user-model".to_owned()))
    );
    assert_eq!(document.get(ENV_KEY), original.get(ENV_KEY));
}

#[test]
fn owned_paths_cover_every_settings_key_betterlite_takes_over() {
    let expected = [
        format!("env.{ENABLE_TOOL_SEARCH_KEY}"),
        format!("env.{GATEWAY_DISCOVERY_KEY}"),
        format!("env.{ANTHROPIC_BASE_URL_KEY}"),
        format!("env.{ANTHROPIC_AUTH_TOKEN_KEY}"),
        format!("env.{ANTHROPIC_API_KEY_KEY}"),
        format!("env.{ANTHROPIC_MODEL_KEY}"),
        API_KEY_HELPER_KEY.to_owned(),
        MODEL_KEY.to_owned(),
        STATUS_LINE_KEY.to_owned(),
    ];
    assert_eq!(OWNED_PATHS, expected);
    assert_eq!(
        CREDENTIAL_PATHS,
        [
            format!("env.{ANTHROPIC_API_KEY_KEY}"),
            format!("env.{ANTHROPIC_AUTH_TOKEN_KEY}"),
            API_KEY_HELPER_KEY.to_owned(),
        ]
    );
    assert_eq!(
        MODEL_PATHS,
        [MODEL_KEY.to_owned(), format!("env.{ANTHROPIC_MODEL_KEY}")]
    );
    assert_eq!(BASE_URL_PATH, format!("env.{ANTHROPIC_BASE_URL_KEY}"));
    assert_eq!(
        TIER_MODEL_ENV_KEYS,
        [
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
        ]
    );
}
