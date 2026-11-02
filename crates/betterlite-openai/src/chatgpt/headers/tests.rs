#![cfg(test)]

use betterlite_config::sections::chatgpt_headers::Settings as HeaderSettings;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};

use super::session::{
    LitellmParamsInput, ModelDumpOutcome, ParamsRecord, SessionIdentity, UuidSessionIdentity,
    ensure_chatgpt_session_id, get_chatgpt_session_id,
};
use super::{
    DEFAULT_ORIGINATOR, DEFAULT_USER_AGENT, PlatformEnvironment, SystemPlatformEnvironment,
    get_chatgpt_default_headers, get_chatgpt_instructions, get_chatgpt_originator,
    get_chatgpt_user_agent,
};

struct FakePlatform;

impl PlatformEnvironment for FakePlatform {
    fn os_type(&self) -> String {
        "Linux".to_owned()
    }

    fn os_release(&self) -> String {
        "6.1.0".to_owned()
    }

    fn arch(&self) -> String {
        "x86_64".to_owned()
    }
}

struct FakeIdentity(&'static str);

impl SessionIdentity for FakeIdentity {
    fn generate(&self) -> String {
        self.0.to_owned()
    }
}

fn settings_with_terminal(terminal: &str) -> HeaderSettings {
    HeaderSettings {
        originator_override: None,
        user_agent_override: None,
        user_agent_suffix: None,
        default_instructions_override: None,
        terminal: terminal.to_owned(),
    }
}

fn mapping(fields: &[(&str, Value)]) -> ParamsRecord {
    let mut record = Map::new();
    for (key, value) in fields {
        record.insert((*key).to_owned(), value.clone());
    }
    record
}

#[test]
fn originator_defaults_to_codex_cli_rs() {
    let settings = HeaderSettings::default();
    assert_eq!(get_chatgpt_originator(&settings), DEFAULT_ORIGINATOR);
}

#[test]
fn originator_sanitizes_control_characters_in_override() {
    let settings = HeaderSettings {
        originator_override: Some("my\tcli".to_owned()),
        user_agent_override: None,
        user_agent_suffix: None,
        default_instructions_override: None,
        terminal: String::new(),
    };
    assert_eq!(get_chatgpt_originator(&settings), "my_cli");
}

#[test]
fn user_agent_uses_sanitized_override_when_set() {
    let settings = HeaderSettings {
        originator_override: None,
        user_agent_override: Some("custom\nagent/1.0".to_owned()),
        user_agent_suffix: None,
        default_instructions_override: None,
        terminal: String::new(),
    };
    assert_eq!(
        get_chatgpt_user_agent(&settings, &FakePlatform, DEFAULT_ORIGINATOR),
        "custom_agent/1.0"
    );
}

#[test]
fn user_agent_sanitizes_every_unprintable_character_in_the_override() {
    let settings = HeaderSettings {
        originator_override: None,
        user_agent_override: Some("\u{0}\u{0}".to_owned()),
        user_agent_suffix: None,
        default_instructions_override: None,
        terminal: String::new(),
    };
    assert_eq!(
        get_chatgpt_user_agent(&settings, &FakePlatform, DEFAULT_ORIGINATOR),
        "__"
    );
}

#[test]
fn user_agent_falls_back_to_default_when_override_is_unset() {
    let settings = HeaderSettings::default();
    assert_ne!(
        get_chatgpt_user_agent(&settings, &FakePlatform, DEFAULT_ORIGINATOR),
        ""
    );
}

#[test]
fn default_user_agent_matches_the_pinned_fallback() {
    assert_eq!(
        DEFAULT_USER_AGENT,
        "codex_cli_rs/0.0.0 (Unknown 0; unknown) unknown"
    );
}

#[test]
fn system_platform_environment_reads_the_real_os_release_and_arch() {
    let platform = SystemPlatformEnvironment;
    assert_eq!(platform.os_type(), "Linux");
    assert_eq!(platform.arch(), std::env::consts::ARCH);
    let uname_release = nix::sys::utsname::uname()
        .expect("uname succeeds on this platform")
        .release()
        .to_str()
        .expect("release is valid UTF-8")
        .to_owned();
    assert_eq!(platform.os_release(), uname_release);
}

#[test]
fn user_agent_combines_originator_version_platform_terminal_and_suffix() {
    let mut settings = settings_with_terminal("iTerm.app/3.5.0");
    settings.user_agent_suffix = Some("  internal build  ".to_owned());
    assert_eq!(
        get_chatgpt_user_agent(&settings, &FakePlatform, "codex_cli_rs"),
        "codex_cli_rs/1.105.1 (Linux 6.1.0; x86_64) iTerm.app/3.5.0 (internal build)"
    );
}

#[test]
fn user_agent_omits_blank_suffix() {
    let settings = settings_with_terminal("unknown");
    assert_eq!(
        get_chatgpt_user_agent(&settings, &FakePlatform, "codex_cli_rs"),
        "codex_cli_rs/1.105.1 (Linux 6.1.0; x86_64) unknown"
    );
}

#[test]
fn default_headers_always_include_core_fields_and_omit_optional_ones() {
    let settings = settings_with_terminal("unknown");
    let token = SecretString::from("secret-token");
    let headers = get_chatgpt_default_headers(&settings, &FakePlatform, &token, None, None);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer secret-token")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("accept").map(String::as_str),
        Some("text/event-stream")
    );
    assert_eq!(
        headers.get("originator").map(String::as_str),
        Some(DEFAULT_ORIGINATOR)
    );
    assert!(!headers.contains_key("session_id"));
    assert!(!headers.contains_key("ChatGPT-Account-Id"));
}

#[test]
fn default_headers_add_session_id_and_account_id_only_for_truthy_inputs() {
    let settings = settings_with_terminal("unknown");
    let token = SecretString::from("secret-token");
    let headers =
        get_chatgpt_default_headers(&settings, &FakePlatform, &token, Some(""), Some("sess-1"));
    assert_eq!(
        headers.get("session_id").map(String::as_str),
        Some("sess-1")
    );
    assert!(!headers.contains_key("ChatGPT-Account-Id"));
}

#[test]
fn access_token_is_not_exposed_through_debug_formatting() {
    let token = SecretString::from("super-secret");
    assert!(!format!("{token:?}").contains("super-secret"));
    assert_eq!(token.expose_secret(), "super-secret");
}

#[test]
fn default_instructions_uses_pinned_prompt_without_override() {
    let settings = HeaderSettings::default();
    let instructions = get_chatgpt_instructions(&settings);
    assert!(instructions.starts_with("You are Codex, based on GPT-5."));
}

#[test]
fn default_instructions_uses_truthy_override() {
    let settings = HeaderSettings {
        originator_override: None,
        user_agent_override: None,
        user_agent_suffix: None,
        default_instructions_override: Some("Stay in character.".to_owned()),
        terminal: String::new(),
    };
    assert_eq!(get_chatgpt_instructions(&settings), "Stay in character.");
}

fn metadata_session_id(value: &str) -> Value {
    Value::Object(mapping(&[("session_id", json!(value))]))
}

#[rstest]
#[case(mapping(&[("litellm_session_id", json!("a")), ("session_id", json!("b"))]), "a")]
#[case(mapping(&[("session_id", json!("b")), ("litellm_trace_id", json!("c"))]), "b")]
#[case(mapping(&[("metadata", metadata_session_id("m")), ("litellm_trace_id", json!("c"))]), "m")]
#[case(mapping(&[("litellm_trace_id", json!("c")), ("litellm_call_id", json!("d"))]), "c")]
#[case(mapping(&[("litellm_call_id", json!("d"))]), "d")]
#[case(mapping(&[("litellm_session_id", json!("")), ("session_id", json!("b"))]), "b")]
#[case(mapping(&[("litellm_session_id", json!(42_i64))]), "42")]
#[case(mapping(&[("litellm_session_id", json!(true))]), "True")]
#[case(mapping(&[("litellm_session_id", json!(["a", "b"]))]), "['a', 'b']")]
#[case(mapping(&[("litellm_session_id", json!({"x": "y"}))]), "{'x': 'y'}")]
fn session_id_precedence_and_stringification(#[case] params: ParamsRecord, #[case] expected: &str) {
    let input = LitellmParamsInput::Mapping(params);
    assert_eq!(get_chatgpt_session_id(&input), Some(expected.to_owned()));
}

#[test]
fn session_id_is_none_when_every_candidate_is_falsy() {
    let params = mapping(&[
        ("litellm_session_id", json!(0_i64)),
        ("session_id", json!(false)),
    ]);
    assert_eq!(
        get_chatgpt_session_id(&LitellmParamsInput::Mapping(params)),
        None
    );
}

#[test]
fn session_id_is_none_for_absent_params() {
    assert_eq!(get_chatgpt_session_id(&LitellmParamsInput::Absent), None);
}

#[test]
fn model_dump_failure_does_not_fall_through_to_a_working_dict_method() {
    let input = LitellmParamsInput::ModelSnapshot {
        model_dump: ModelDumpOutcome::Failed,
        dict: ModelDumpOutcome::Succeeded(mapping(&[("litellm_session_id", json!("from-dict"))])),
    };
    assert_eq!(get_chatgpt_session_id(&input), None);
}

#[test]
fn model_dump_missing_falls_back_to_dict_method() {
    let input = LitellmParamsInput::ModelSnapshot {
        model_dump: ModelDumpOutcome::Unavailable,
        dict: ModelDumpOutcome::Succeeded(mapping(&[("litellm_session_id", json!("from-dict"))])),
    };
    assert_eq!(get_chatgpt_session_id(&input), Some("from-dict".to_owned()));
}

#[test]
fn ensure_chatgpt_session_id_returns_the_selected_session() {
    let input = LitellmParamsInput::Mapping(mapping(&[("session_id", json!("chosen"))]));
    assert_eq!(
        ensure_chatgpt_session_id(&input, &FakeIdentity("fallback")),
        "chosen"
    );
}

#[test]
fn ensure_chatgpt_session_id_falls_back_to_the_injected_identity() {
    assert_eq!(
        ensure_chatgpt_session_id(&LitellmParamsInput::Absent, &FakeIdentity("fallback")),
        "fallback"
    );
}

#[test]
fn uuid_session_identity_generates_a_random_v4_uuid() {
    let generated = UuidSessionIdentity.generate();
    let parsed = uuid::Uuid::parse_str(&generated).expect("a valid uuid string");
    assert_eq!(parsed.get_version_num(), 4);
}
