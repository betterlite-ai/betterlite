#![cfg(test)]

use rstest::rstest;

use super::{
    ClaudePluginWriteAction, ClaudePluginWriteFailure, ClaudePluginWriteInput,
    ClaudePluginWriteResult, PLUGIN_NAME_INVALID, PluginMessage, PluginRegistration, PluginView,
    RegistrationAction, SUCCESS,
};
use crate::api::claude_plugin_policy::ClaudePluginPolicyError;
use crate::api::claude_plugin_policy::source::{
    GITHUB_REPO_MISSING, PluginSource, PluginSpec, UNKNOWN_SOURCE,
};

const GITHUB_SPEC: &str = r#"{"source":{"source":"github","repo":"org/plugin"}}"#;

fn spec(json: &str) -> PluginSpec {
    serde_json::from_str(json).expect("the plugin spec")
}

fn source(json: &str) -> PluginSource {
    serde_json::from_str(json).expect("the plugin source")
}

fn input(
    action: ClaudePluginWriteAction,
    plugin_name: &str,
    plugin_spec: Option<PluginSpec>,
) -> ClaudePluginWriteInput {
    ClaudePluginWriteInput {
        action,
        plugin_name: plugin_name.to_owned(),
        spec: plugin_spec,
    }
}

fn name_error(write: &ClaudePluginWriteInput) {
    match write.validate() {
        Err(ClaudePluginWriteFailure::Policy(ClaudePluginPolicyError::Response {
            status_code,
            message,
        })) => {
            assert_eq!(status_code, 400);
            assert_eq!(message, PLUGIN_NAME_INVALID);
        }
        other => panic!("expected the 400 plugin-name error, got {other:?}"),
    }
}

fn source_error(write: &ClaudePluginWriteInput, expected: &str) {
    match write.validate() {
        Err(ClaudePluginWriteFailure::Policy(ClaudePluginPolicyError::InvalidSource {
            message,
        })) => assert_eq!(message, expected),
        other => panic!("expected the 400 source error, got {other:?}"),
    }
}

#[rstest]
#[case::plain("plugin")]
#[case::numbered("plugin-2")]
#[case::hyphenated("my-cool-plugin")]
#[case::digits("9-lives")]
fn create_accepts_kebab_names(#[case] name: &str) {
    let write = input(
        ClaudePluginWriteAction::Create,
        name,
        Some(spec(GITHUB_SPEC)),
    );
    write.validate().unwrap();
    assert_eq!(write.manifest().unwrap().name, name);
}

#[rstest]
#[case::underscore("my_plugin")]
#[case::uppercase("MyPlugin")]
#[case::space("my plugin")]
#[case::empty("")]
#[case::accented("plügin")]
#[case::slash("org/plugin")]
fn create_rejects_other_names(#[case] name: &str) {
    let write = input(
        ClaudePluginWriteAction::Create,
        name,
        Some(spec(GITHUB_SPEC)),
    );
    name_error(&write);
}

#[test]
fn update_validates_only_the_source() {
    let valid = input(
        ClaudePluginWriteAction::Update,
        "Bad_Name",
        Some(spec(GITHUB_SPEC)),
    );
    valid.validate().unwrap();
    let invalid = input(
        ClaudePluginWriteAction::Update,
        "Bad_Name",
        Some(spec(r#"{"source":{"source":"github"}}"#)),
    );
    source_error(&invalid, GITHUB_REPO_MISSING);
}

#[rstest]
#[case::create(ClaudePluginWriteAction::Create)]
#[case::update(ClaudePluginWriteAction::Update)]
fn unknown_sources_are_rejected(#[case] action: ClaudePluginWriteAction) {
    let write = input(
        action,
        "fine-name",
        Some(spec(r#"{"source":{"source":"ftp"}}"#)),
    );
    source_error(&write, UNKNOWN_SOURCE);
}

#[rstest]
#[case::enable(ClaudePluginWriteAction::Enable)]
#[case::disable(ClaudePluginWriteAction::Disable)]
#[case::delete(ClaudePluginWriteAction::Delete)]
fn lifecycle_actions_need_no_spec(#[case] action: ClaudePluginWriteAction) {
    let write = input(action, "any-name", None);
    write.validate().unwrap();
    assert!(matches!(
        write.manifest(),
        Err(ClaudePluginWriteFailure::MissingSpec)
    ));
    assert!(matches!(
        write.fields(),
        Err(ClaudePluginWriteFailure::MissingSpec)
    ));
}

#[rstest]
#[case::create(ClaudePluginWriteAction::Create)]
#[case::update(ClaudePluginWriteAction::Update)]
fn writes_need_a_spec(#[case] action: ClaudePluginWriteAction) {
    let write = input(action, "fine-name", None);
    assert!(matches!(
        write.validate(),
        Err(ClaudePluginWriteFailure::MissingSpec)
    ));
}

#[test]
fn manifest_uses_the_default_version() {
    let write = input(
        ClaudePluginWriteAction::Create,
        "minimal-plugin",
        Some(spec(GITHUB_SPEC)),
    );
    let manifest = write.manifest().unwrap();
    assert_eq!(manifest.version.as_deref(), Some("1.0.0"));
    assert_eq!(
        serde_json::to_string(&manifest).unwrap(),
        r#"{"name":"minimal-plugin","source":{"source":"github","repo":"org/plugin"},"version":"1.0.0"}"#
    );
}

#[test]
fn manifest_drops_empty_fields() {
    let write = input(
        ClaudePluginWriteAction::Create,
        "empty-plugin",
        Some(spec(
            r#"{"source":{"source":"github","repo":"org/plugin"},"version":"","description":"","homepage":"","keywords":[],"category":"","domain":"","namespace":""}"#,
        )),
    );
    let manifest = write.manifest().unwrap();
    assert_eq!(manifest.version, None);
    assert_eq!(
        serde_json::to_string(&manifest).unwrap(),
        r#"{"name":"empty-plugin","source":{"source":"github","repo":"org/plugin"}}"#
    );
}

#[test]
fn fields_carry_the_spec() {
    let write = input(
        ClaudePluginWriteAction::Create,
        "field-plugin",
        Some(spec(
            r#"{"source":{"source":"url","url":"https://github.com/org/repo.git"},"version":"2.0.0","description":"a plugin"}"#,
        )),
    );
    let fields = write.fields().unwrap();
    assert_eq!(fields.version.as_deref(), Some("2.0.0"));
    assert_eq!(fields.description.as_deref(), Some("a plugin"));
    assert_eq!(
        fields.source,
        source(r#"{"source":"url","url":"https://github.com/org/repo.git"}"#)
    );
}

#[test]
fn results_serialize_as_litellm_does() {
    let registration = ClaudePluginWriteResult::Registration(PluginRegistration {
        status: SUCCESS,
        action: RegistrationAction::Created,
        plugin: PluginView {
            id: "plugin-1".to_owned(),
            name: "my-plugin".to_owned(),
            version: Some("1.0.0".to_owned()),
            description: None,
            source: source(r#"{"source":"github","repo":"org/my-plugin"}"#),
            enabled: true,
        },
    });
    assert_eq!(
        serde_json::to_string(&registration).unwrap(),
        r#"{"status":"success","action":"created","plugin":{"id":"plugin-1","name":"my-plugin","version":"1.0.0","description":null,"source":{"source":"github","repo":"org/my-plugin"},"enabled":true}}"#
    );
    let message = ClaudePluginWriteResult::Message(PluginMessage {
        status: SUCCESS,
        message: "Plugin 'my-plugin' deleted".to_owned(),
    });
    assert_eq!(
        serde_json::to_string(&message).unwrap(),
        r#"{"status":"success","message":"Plugin 'my-plugin' deleted"}"#
    );
    assert_eq!(
        serde_json::to_string(&RegistrationAction::Updated).unwrap(),
        r#""updated""#
    );
}
