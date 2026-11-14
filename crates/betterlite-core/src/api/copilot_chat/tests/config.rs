use betterlite_config::sections::copilot_chat::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use super::support::{FakeCredentials, FakePredicates, KeyOutcome, config};
use crate::api::copilot_chat::error::CopilotChatError;
use crate::api::copilot_chat::messages::CopilotMessage;

const REQUEST_ID: Uuid = Uuid::from_u128(7);

fn settings_with_base(api_base: Option<&str>) -> Settings {
    Settings {
        disable_system_to_assistant: false,
        api_base: api_base.map(String::from),
    }
}

fn messages(text: &str) -> Vec<CopilotMessage> {
    serde_json::from_str(text).unwrap()
}

#[rstest::rstest]
#[case::explicit_wins(
    Some("https://explicit.test"),
    Some("https://cached.test"),
    Some("https://env.test"),
    "https://explicit.test"
)]
#[case::explicit_empty_is_used_as_given(Some(""), Some("https://cached.test"), None, "")]
#[case::cached_endpoint_beats_the_environment(
    None,
    Some("https://cached.test"),
    Some("https://env.test"),
    "https://cached.test"
)]
#[case::environment_beats_the_default(None, None, Some("https://env.test"), "https://env.test")]
#[case::default_base(None, None, None, "https://api.githubcopilot.com")]
#[tokio::test]
async fn api_base_without_login_resolves_explicit_cached_environment_then_default(
    #[case] explicit: Option<&str>,
    #[case] cached: Option<&'static str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let credentials = FakeCredentials {
        base: cached,
        ..FakeCredentials::default()
    };
    let mut copilot = config(
        credentials,
        FakePredicates::default(),
        settings_with_base(environment),
    );
    let base = copilot.api_base_without_login(explicit).await;
    assert_eq!(base, expected);
    assert_eq!(copilot.credentials.key_calls, 0);
    assert_eq!(
        copilot.credentials.base_calls,
        usize::from(explicit.is_none())
    );
}

#[tokio::test]
async fn provider_info_ignores_the_supplied_key_and_takes_the_authenticator_key() {
    let credentials = FakeCredentials {
        key: KeyOutcome::Token("copilot-key"),
        ..FakeCredentials::default()
    };
    let mut copilot = config(
        credentials,
        FakePredicates::default(),
        settings_with_base(None),
    );
    let info = copilot
        .provider_info("gpt-4o", None, Some("supplied-key"), "github_copilot")
        .await
        .unwrap();
    assert_eq!(info.api_base, "https://api.githubcopilot.com");
    assert_eq!(info.provider, "github_copilot");
    assert_eq!(info.api_key.unwrap().expose_secret(), "copilot-key");
}

#[tokio::test]
async fn provider_info_translates_an_api_key_failure_to_an_authentication_error() {
    let credentials = FakeCredentials {
        key: KeyOutcome::Refresh,
        ..FakeCredentials::default()
    };
    let mut copilot = config(
        credentials,
        FakePredicates::default(),
        settings_with_base(None),
    );
    let error = copilot
        .provider_info("gpt-4o", Some("https://given.test"), None, "github_copilot")
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(401));
    assert_eq!(
        error.to_string(),
        "no Copilot API key after three refresh attempts"
    );
    assert!(matches!(
        &error,
        CopilotChatError::Authentication { model, provider, .. }
            if model == "gpt-4o" && provider == "github_copilot"
    ));
}

#[tokio::test]
async fn provider_info_lets_a_login_failure_keep_its_own_status() {
    let credentials = FakeCredentials {
        key: KeyOutcome::Login { status: 503 },
        ..FakeCredentials::default()
    };
    let mut copilot = config(
        credentials,
        FakePredicates::default(),
        settings_with_base(None),
    );
    let error = copilot
        .provider_info("gpt-4o", None, None, "github_copilot")
        .await
        .unwrap_err();
    assert!(matches!(error, CopilotChatError::Login(_)));
    assert_eq!(error.status(), Some(503));
}

#[tokio::test]
async fn validate_environment_lets_the_parent_headers_win_over_the_copilot_defaults() {
    let credentials = FakeCredentials {
        key: KeyOutcome::Token("copilot-key"),
        ..FakeCredentials::default()
    };
    let mut copilot = config(
        credentials,
        FakePredicates::default(),
        settings_with_base(None),
    );
    copilot.base.api_key = Some(SecretString::from(String::from("caller-key")));
    copilot.base.headers.insert(
        String::from("editor-version"),
        String::from("caller-editor"),
    );
    let headers = copilot.validate_environment(&[], REQUEST_ID).await.unwrap();
    assert_eq!(headers["Authorization"], "Bearer caller-key");
    assert_eq!(headers["editor-version"], "caller-editor");
    assert_eq!(headers["copilot-integration-id"], "vscode-chat");
    assert_eq!(headers["x-request-id"], REQUEST_ID.to_string());
    assert_eq!(headers["X-Initiator"], "user");
}

#[tokio::test]
async fn validate_environment_without_a_key_keeps_only_the_parent_headers() {
    let credentials = FakeCredentials {
        key: KeyOutcome::Refresh,
        ..FakeCredentials::default()
    };
    let mut copilot = config(
        credentials,
        FakePredicates::default(),
        settings_with_base(None),
    );
    let headers = copilot.validate_environment(&[], REQUEST_ID).await.unwrap();
    let names: Vec<&str> = headers.keys().map(String::as_str).collect();
    assert_eq!(names, ["Authorization", "X-Initiator"]);
    assert_eq!(headers["Authorization"], "Bearer None");
}

#[tokio::test]
async fn validate_environment_propagates_a_login_failure() {
    let credentials = FakeCredentials {
        key: KeyOutcome::Login { status: 503 },
        ..FakeCredentials::default()
    };
    let mut copilot = config(
        credentials,
        FakePredicates::default(),
        settings_with_base(None),
    );
    let error = copilot
        .validate_environment(&[], REQUEST_ID)
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(503));
}

#[rstest::rstest]
#[case::no_messages("[]", "user")]
#[case::user_and_system_only(r#"[{"role":"system"},{"role":"user"}]"#, "user")]
#[case::assistant(r#"[{"role":"user"},{"role":"assistant"}]"#, "agent")]
#[case::tool(r#"[{"role":"tool"}]"#, "agent")]
#[case::missing_role(r#"[{"content":"x"}]"#, "user")]
#[tokio::test]
async fn validate_environment_overwrites_the_initiator_from_the_message_roles(
    #[case] raw: &str,
    #[case] expected: &str,
) {
    let mut copilot = config(
        FakeCredentials::default(),
        FakePredicates::default(),
        settings_with_base(None),
    );
    copilot
        .base
        .headers
        .insert(String::from("X-Initiator"), String::from("caller"));
    let headers = copilot
        .validate_environment(&messages(raw), REQUEST_ID)
        .await
        .unwrap();
    assert_eq!(headers["X-Initiator"], expected);
}

#[rstest::rstest]
#[case::type_and_url(
    r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"u"}}]}]"#,
    true
)]
#[case::key_without_type(r#"[{"role":"user","content":[{"image_url":{"url":"u"}}]}]"#, true)]
#[case::type_without_key(r#"[{"role":"user","content":[{"type":"image_url"}]}]"#, true)]
#[case::text_part(r#"[{"role":"user","content":[{"type":"text","text":"t"}]}]"#, false)]
#[case::string_content(r#"[{"role":"user","content":"image_url"}]"#, false)]
#[case::no_content(r#"[{"role":"user"}]"#, false)]
#[case::later_message(
    r#"[{"role":"user","content":"x"},{"role":"user","content":[{"image_url":{}}]}]"#,
    true
)]
#[tokio::test]
async fn validate_environment_marks_vision_requests_by_an_image_key_or_type(
    #[case] raw: &str,
    #[case] vision: bool,
) {
    let mut copilot = config(
        FakeCredentials::default(),
        FakePredicates::default(),
        settings_with_base(None),
    );
    let headers = copilot
        .validate_environment(&messages(raw), REQUEST_ID)
        .await
        .unwrap();
    assert_eq!(
        headers.get("Copilot-Vision-Request").map(String::as_str),
        vision.then_some("true")
    );
}

#[rstest::rstest]
#[case::claude_reasoning("claude-sonnet-4", &["claude-sonnet-4"], true)]
#[case::mixed_case_is_lowered("Claude-Sonnet-4", &["claude-sonnet-4"], true)]
#[case::claude_without_reasoning("claude-3-haiku", &["claude-sonnet-4"], false)]
#[case::other_reasoning_model("o3-mini", &["o3-mini"], false)]
#[tokio::test]
async fn supported_params_add_thinking_for_reasoning_claude_names_only(
    #[case] model: &str,
    #[case] reasoning: &[&'static str],
    #[case] added: bool,
) {
    let predicates = FakePredicates {
        reasoning: reasoning.to_vec(),
        ..FakePredicates::default()
    };
    let mut copilot = config(
        FakeCredentials::default(),
        predicates,
        settings_with_base(None),
    );
    let params = copilot.supported_params(model).await;
    assert_eq!(params.contains(&"thinking"), added);
    assert_eq!(params.contains(&"reasoning_effort"), added);
    assert!(params.contains(&"temperature"));
    assert_eq!(
        copilot.predicates.asked.is_empty(),
        !model.to_lowercase().contains("claude")
    );
}
