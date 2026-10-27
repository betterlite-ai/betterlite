#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use betterlite_harness::context::ApprovalHandler;
use betterlite_harness::errors::HarnessError;
use betterlite_harness::options::{ClaudeCodeOptions, CodexOptions, HarnessOptions};
use betterlite_harness::session::output::{OutputError, parse_output};
use betterlite_harness::session::routing::{ProxySettings, RouteError, resolve_model_route};
use betterlite_harness::session::skills::{SkillError, normalize_skill};
use betterlite_harness::session::validation::{
    PERMISSION_MODES, ValidationError, check_basic, check_capabilities, validate,
};
use betterlite_harness::session::{SessionConfig, SessionParameters, build_config, context_for};
use betterlite_harness::transformation::BaseHarnessConfig;
use betterlite_harness::types::{Capabilities, Capability, Harness, JsonObject, PermissionMode};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct Answer {
    value: u32,
}

#[rstest]
#[case::last(None, "progress\n{\"value\": 1}\n{\"value\": 7}")]
#[case::structured(Some("{\"value\": 7}"), "{\"value\": 2}")]
#[case::empty(Some(""), "{\"value\": 7}")]
fn parses_the_final_json_object(#[case] structured: Option<&str>, #[case] text: &str) {
    let answer = parse_output::<Answer>(structured, text).unwrap();
    assert_eq!(answer, Answer { value: 7 });
}

#[rstest]
#[case::missing(None, "plain", "no JSON object found in the final answer")]
#[case::malformed(Some("{"), "{\"value\": 7}", "EOF")]
#[case::wrong_type(Some("{\"value\": true}"), "", "invalid type")]
#[case::missing_field(Some("{}"), "", "missing field")]
fn rejects_invalid_output(
    #[case] structured: Option<&str>,
    #[case] text: &str,
    #[case] message: &str,
) {
    let error = parse_output::<Answer>(structured, text).unwrap_err();
    assert!(error.to_string().contains(message));
    if message.starts_with("no JSON") {
        assert!(matches!(error, OutputError::NoJsonObject));
    } else {
        assert!(matches!(error, OutputError::Invalid(_)));
    }
}

#[test]
fn strips_proxy_prefix_and_gateway_slash() {
    let key = SecretString::from(" key ");
    let (model, gateway) = resolve_model_route(
        Some("litellm_proxy/team"),
        Some(&key),
        Some(" https://gateway/// "),
        &ProxySettings::default(),
    )
    .unwrap();
    let target = gateway.unwrap();
    assert_eq!(model.as_deref(), Some("team"));
    assert_eq!(target.api_base, "https://gateway");
    assert_eq!(target.api_key.expose_secret(), "key");
    assert!(!format!("{target:?}").contains("\"key\""));
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(""))]
#[case::direct(Some("provider/model"))]
#[case::not_a_prefix(Some("x/litellm_proxy/team"))]
fn leaves_direct_routes_unchanged(#[case] model: Option<&str>) {
    let (routed, gateway) =
        resolve_model_route(model, None, None, &ProxySettings::default()).unwrap();
    assert_eq!(routed.as_deref(), model);
    assert!(gateway.is_none());
}

#[rstest]
#[case::none(None, None)]
#[case::direct(Some("team"), Some("team"))]
#[case::prefixed(Some("litellm_proxy/\u{7ec4}"), Some("\u{7ec4}"))]
#[case::empty_group(Some("litellm_proxy/"), Some(""))]
fn uses_injected_proxy_settings(#[case] model: Option<&str>, #[case] expected: Option<&str>) {
    let proxy = ProxySettings {
        use_litellm_proxy: true,
        api_base: Some(" https://env/// ".to_owned()),
        api_key: Some(SecretString::from(" env-key ")),
    };
    let (routed, gateway) = resolve_model_route(model, None, None, &proxy).unwrap();
    let target = gateway.unwrap();
    assert_eq!(routed.as_deref(), expected);
    assert_eq!(target.api_base, "https://env");
    assert_eq!(target.api_key.expose_secret(), "env-key");
}

#[rstest]
#[case::explicit(
    "https://explicit/",
    "explicit-key",
    "https://explicit",
    "explicit-key"
)]
#[case::empty("", "", "https://env", "env-key")]
fn applies_route_precedence(
    #[case] base: &str,
    #[case] key: &str,
    #[case] expected_base: &str,
    #[case] expected_key: &str,
) {
    let proxy = ProxySettings {
        use_litellm_proxy: true,
        api_base: Some("https://env/".to_owned()),
        api_key: Some(SecretString::from("env-key")),
    };
    let secret = SecretString::from(key);
    let (_, gateway) = resolve_model_route(None, Some(&secret), Some(base), &proxy).unwrap();
    let target = gateway.unwrap();
    assert_eq!(target.api_base, expected_base);
    assert_eq!(target.api_key.expose_secret(), expected_key);
}

#[rstest]
#[case::no_base(None, None, true)]
#[case::blank_base(Some(" "), Some("key"), true)]
#[case::no_key(Some("https://gateway"), None, false)]
#[case::blank_key(Some("https://gateway"), Some(" "), false)]
fn rejects_missing_route_values(
    #[case] base: Option<&str>,
    #[case] key: Option<&str>,
    #[case] missing_base: bool,
) {
    let secret = key.map(SecretString::from);
    let error = resolve_model_route(
        Some("litellm_proxy/team"),
        secret.as_ref(),
        base,
        &ProxySettings::default(),
    )
    .unwrap_err();
    assert_eq!(matches!(error, RouteError::MissingBase), missing_base);
    assert!(error.to_string().contains("LITELLM_PROXY_API_"));
}

fn configuration() -> SessionConfig<()> {
    let parameters = SessionParameters::default();
    SessionConfig {
        harness: Harness::Codex,
        sandbox: (),
        parameters,
    }
}

#[test]
fn rejects_invalid_basic_settings() {
    let mut config = configuration();
    config.parameters.settings.max_turns = Some(0);
    assert_eq!(
        check_basic(&config).unwrap_err().to_string(),
        "max_turns must be >= 1"
    );
    config.parameters.settings.max_turns = Some(1);
    config.parameters.settings.timeout = Some(Duration::ZERO);
    assert!(matches!(
        check_basic(&config),
        Err(ValidationError::Timeout)
    ));
    config.parameters.settings.timeout = Some(Duration::from_nanos(1));
    check_basic(&config).unwrap();
    config.parameters.install = true;
    assert!(matches!(
        check_basic(&config),
        Err(ValidationError::InstallUnsupported)
    ));
}

#[test]
fn permission_whitelist_is_exact() {
    let modes: Vec<String> = PERMISSION_MODES
        .iter()
        .map(|mode| serde_json::to_string(mode).unwrap())
        .collect();
    assert_eq!(modes, ["\"full\"", "\"read-only\"", "\"ask\"", "\"edit\""]);
    serde_json::from_str::<PermissionMode>("\"FULL\"").unwrap_err();
    serde_json::from_str::<PermissionMode>("\"unknown\"").unwrap_err();
}

#[rstest]
#[case::output(Capability::StructuredOutput)]
#[case::skills(Capability::Skills)]
#[case::filter(Capability::ToolFiltering)]
fn guards_optional_capabilities(#[case] capability: Capability) {
    let mut config = configuration();
    match capability {
        Capability::StructuredOutput => {
            config.parameters.settings.output_schema = Some(JsonObject::new());
        }
        Capability::Skills => config.parameters.settings.skills.push("skill".to_owned()),
        Capability::ToolFiltering => config
            .parameters
            .settings
            .disable_tools
            .push("shell".to_owned()),
        Capability::ToolApproval
        | Capability::CustomTools
        | Capability::History
        | Capability::Resume => {
            panic!("not a session option")
        }
    }
    let mut capabilities = Capabilities {
        supported: BTreeSet::new(),
        permission_modes: BTreeSet::from([PermissionMode::Full]),
    };
    let error = check_capabilities(&config, &capabilities, false).unwrap_err();
    assert!(
        matches!(error, ValidationError::Harness(HarnessError::CapabilityUnsupported { capability: actual, .. }) if actual == capability)
    );
    capabilities.supported.insert(capability);
    check_capabilities(&config, &capabilities, false).unwrap();
}

#[test]
fn guards_interactive_approval() {
    let mut config = configuration();
    config.parameters.settings.permissions = PermissionMode::Ask;
    let mut capabilities = Capabilities {
        supported: BTreeSet::new(),
        permission_modes: BTreeSet::new(),
    };
    assert!(matches!(
        check_capabilities(&config, &capabilities, true),
        Err(ValidationError::PermissionUnsupported { .. })
    ));
    capabilities.permission_modes.insert(PermissionMode::Ask);
    assert!(matches!(
        check_capabilities(&config, &capabilities, true),
        Err(ValidationError::Harness(
            HarnessError::CapabilityUnsupported {
                capability: Capability::ToolApproval,
                ..
            }
        ))
    ));
    capabilities.supported.insert(Capability::ToolApproval);
    assert!(matches!(
        check_capabilities(&config, &capabilities, false),
        Err(ValidationError::ApprovalRequired)
    ));
    check_capabilities(&config, &capabilities, true).unwrap();
}

#[test]
fn asks_for_a_handler_before_other_capabilities() {
    let mut config = configuration();
    config.parameters.settings.permissions = PermissionMode::Ask;
    config.parameters.settings.output_schema = Some(JsonObject::new());
    let capabilities = Capabilities {
        supported: BTreeSet::from([Capability::ToolApproval]),
        permission_modes: BTreeSet::from([PermissionMode::Ask]),
    };
    assert!(matches!(
        check_capabilities(&config, &capabilities, false),
        Err(ValidationError::ApprovalRequired)
    ));
    let handler: ApprovalHandler = Arc::new(|_| Box::pin(async { true }));
    config.parameters.settings.on_approval = Some(handler);
    assert!(matches!(
        check_capabilities(&config, &capabilities, false),
        Err(ValidationError::Harness(
            HarnessError::CapabilityUnsupported {
                capability: Capability::StructuredOutput,
                ..
            }
        ))
    ));
}

struct CodexConfig;

impl BaseHarnessConfig for CodexConfig {
    type Options = CodexOptions;
    const HARNESS: Harness = Harness::Codex;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supported: BTreeSet::new(),
            permission_modes: BTreeSet::from([PermissionMode::Full]),
        }
    }
}

#[test]
fn validates_in_order() {
    let mut config = configuration();
    config.parameters.settings.max_turns = Some(0);
    config.parameters.settings.options =
        Some(HarnessOptions::ClaudeCode(ClaudeCodeOptions::default()));
    config.parameters.settings.permissions = PermissionMode::Ask;
    assert!(matches!(
        validate(&config, &CodexConfig, false),
        Err(ValidationError::MaxTurns)
    ));
    config.parameters.settings.max_turns = Some(1);
    assert!(matches!(
        validate(&config, &CodexConfig, false),
        Err(ValidationError::Harness(
            HarnessError::OptionsMismatch { .. }
        ))
    ));
    config.parameters.settings.options = Some(HarnessOptions::Codex(CodexOptions::default()));
    assert!(matches!(
        validate(&config, &CodexConfig, false),
        Err(ValidationError::PermissionUnsupported { .. })
    ));
    config.parameters.settings.permissions = PermissionMode::Full;
    validate(&config, &CodexConfig, false).unwrap();
    config.parameters.settings.options = None;
    validate(&config, &CodexConfig, false).unwrap();
}

#[test]
fn builds_defaults_and_context() {
    let config = build_config(
        Harness::Codex,
        (),
        SessionParameters::default(),
        &ProxySettings::default(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .unwrap();
    assert!(!config.parameters.install);
    validate(&config, &CodexConfig, false).unwrap();
    let session_id = Uuid::new_v4();
    let context = context_for(&config, session_id);
    assert_eq!(context.harness, Harness::Codex);
    assert_eq!(context.session_id, session_id.as_simple().to_string());
    assert_eq!(context.session_id.len(), 32);
    assert_eq!(context.settings.permissions, PermissionMode::Full);
    assert!(context.settings.model.is_none());
    assert!(context.settings.gateway.is_none());
    assert!(context.settings.skills.is_empty());
    assert!(context.settings.max_turns.is_none());
    assert!(context.settings.timeout.is_none());
    assert!(context.settings.metadata.is_empty());
    assert_eq!(context.outcome.usage.calls, 0);
}

#[test]
fn rejects_missing_skill_manifest() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(matches!(
        normalize_skill(Path::new("src"), directory),
        Err(SkillError::NoManifest(_))
    ));
    assert!(matches!(
        normalize_skill(Path::new("src"), Path::new("relative")),
        Err(SkillError::RelativeDirectory)
    ));
}

#[test]
fn normalizes_and_preserves_skills() {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(Uuid::new_v4().to_string());
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("SKILL.md"), "skill").unwrap();
    let mut parameters = SessionParameters::default();
    parameters.settings.skills = vec!["unused/..".to_owned(), ".".to_owned()];
    let config = build_config(
        Harness::Codex,
        (),
        parameters,
        &ProxySettings::default(),
        &folder,
    )
    .unwrap();
    let expected = folder.to_str().unwrap();
    assert_eq!(config.parameters.settings.skills, [expected, expected]);
    let context = context_for(&config, Uuid::new_v4());
    assert_eq!(context.settings.skills, [expected, expected]);
    fs::remove_dir_all(&folder).unwrap();
}
