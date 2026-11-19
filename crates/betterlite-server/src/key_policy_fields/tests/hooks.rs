#![cfg(test)]

use std::path::PathBuf;

use axum::http::StatusCode;
use tempfile::TempDir;

use super::{object, update_request};
use crate::key_fields::decode::DecodeError;
use crate::key_fields::policy::{CustomKeyPolicyRequest, PolicyOperation, PolicyRequestBody};
use crate::key_policy_fields::columns::{update_policy_request, verification_token_from_row};
use crate::key_policy_fields::hooks::{
    DEFAULT_DENIAL, HookPolicyError, KeyHooks, POLICY_NOT_COROUTINE, UPDATE_NOT_COROUTINE,
    enforce_key_policy_hook, enforce_key_update_hook,
};
use crate::python_hooks::PythonHooks;
use crate::python_hooks::slots::{HookSlot, LoadedHook};

const UPDATE_HOOKS: &str = r#"
async def update_default(data):
    return {}

async def update_allow(data):
    return {"decision": True}

async def update_deny(data):
    return {"decision": False}

async def update_deny_custom(data):
    return {"decision": False, "message": "no keys for you"}

async def update_null_decision(data):
    return {"decision": None}

async def update_deny_null_message(data):
    return {"decision": False, "message": None}

async def update_deny_bad_message(data):
    return {"decision": False, "message": 5}

async def update_bad_decision(data):
    return {"decision": "yes"}

async def update_not_mapping(data):
    return [1]

def update_sync(data):
    return {}

async def update_broken(data):
    raise RuntimeError("boom")

async def generate_hook(data):
    return {}

async def policy_sync(data):
    return {}
"#;

struct Workspace {
    _directory: TempDir,
    config: PathBuf,
}

fn workspace() -> Workspace {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.yaml");
    std::fs::write(&config, b"model_list: []\n").unwrap();
    std::fs::write(directory.path().join("b1224_hooks.py"), UPDATE_HOOKS).unwrap();
    Workspace {
        _directory: directory,
        config,
    }
}

async fn load(service: &PythonHooks, slot: HookSlot, name: &str) -> LoadedHook {
    service
        .load_hook(slot, &format!("b1224_hooks.{name}"))
        .await
        .unwrap()
}

#[tokio::test]
async fn hook_getters() {
    let workspace = workspace();
    let service = PythonHooks::new(Some(workspace.config));
    let generate = load(&service, HookSlot::CustomKeyGenerate, "generate_hook").await;
    let update = load(&service, HookSlot::CustomKeyUpdate, "update_default").await;
    let policy = load(&service, HookSlot::CustomKeyPolicy, "update_default").await;
    let hooks = KeyHooks {
        generate: Some(generate),
        update: Some(update),
        policy: Some(policy),
    };
    assert_eq!(
        hooks.generate.as_ref().map(|hook| hook.value.as_str()),
        Some("b1224_hooks.generate_hook")
    );
    assert_eq!(
        hooks.update.as_ref().map(|hook| hook.value.as_str()),
        Some("b1224_hooks.update_default")
    );
    assert_eq!(
        hooks.policy.as_ref().map(|hook| hook.value.as_str()),
        Some("b1224_hooks.update_default")
    );
    let absent = KeyHooks::default();
    assert!(absent.generate.is_none() && absent.update.is_none() && absent.policy.is_none());
    service.shutdown().await;
}

#[tokio::test]
async fn missing_hooks_noop() {
    let data = update_request(r#"{"key_alias":"alias"}"#);
    assert!(enforce_key_update_hook(None, &data).await.is_ok());
    assert!(
        enforce_key_policy_hook(None, || panic!("the builder must not run"))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn synchronous_hook_rejected_before_builder() {
    let workspace = workspace();
    let service = PythonHooks::new(Some(workspace.config));
    let sync_update = load(&service, HookSlot::CustomKeyUpdate, "update_sync").await;
    let data = update_request(r#"{"key_alias":"alias"}"#);
    let error = enforce_key_update_hook(Some(&sync_update), &data)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        HookPolicyError::NotCoroutine { message } if *message == UPDATE_NOT_COROUTINE
    ));
    assert_eq!(error.to_string(), UPDATE_NOT_COROUTINE);
    assert_eq!(error.status_code(), None);
    let sync_policy = load(&service, HookSlot::CustomKeyPolicy, "policy_sync").await;
    let error = enforce_key_policy_hook(Some(&sync_policy), || panic!("the builder must not run"))
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        HookPolicyError::NotCoroutine { message } if *message == POLICY_NOT_COROUTINE
    ));
    service.shutdown().await;
}

#[tokio::test]
async fn decision_default_and_custom_denial() {
    let workspace = workspace();
    let service = PythonHooks::new(Some(workspace.config));
    let data = update_request(r#"{"key_alias":"alias"}"#);
    let default_allow = load(&service, HookSlot::CustomKeyUpdate, "update_default").await;
    assert!(
        enforce_key_update_hook(Some(&default_allow), &data)
            .await
            .is_ok()
    );
    let explicit_allow = load(&service, HookSlot::CustomKeyUpdate, "update_allow").await;
    assert!(
        enforce_key_update_hook(Some(&explicit_allow), &data)
            .await
            .is_ok()
    );
    let deny = load(&service, HookSlot::CustomKeyUpdate, "update_deny").await;
    let error = enforce_key_update_hook(Some(&deny), &data)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        HookPolicyError::Denied { message } if message == DEFAULT_DENIAL
    ));
    assert_eq!(error.status_code(), Some(StatusCode::FORBIDDEN));
    let custom = load(&service, HookSlot::CustomKeyUpdate, "update_deny_custom").await;
    let error = enforce_key_update_hook(Some(&custom), &data)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        HookPolicyError::Denied { message } if message == "no keys for you"
    ));
    let null_decision = load(&service, HookSlot::CustomKeyUpdate, "update_null_decision").await;
    assert!(
        enforce_key_update_hook(Some(&null_decision), &data)
            .await
            .is_ok(),
        "a null decision takes the default"
    );
    let null_message = load(
        &service,
        HookSlot::CustomKeyUpdate,
        "update_deny_null_message",
    )
    .await;
    let error = enforce_key_update_hook(Some(&null_message), &data)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        HookPolicyError::Denied { message } if message == DEFAULT_DENIAL
    ));
    let policy = load(&service, HookSlot::CustomKeyPolicy, "update_allow").await;
    let allowed = enforce_key_policy_hook(Some(&policy), policy_request).await;
    assert!(
        allowed.is_ok(),
        "an allowing policy hook passes: {allowed:?}"
    );
    service.shutdown().await;
}

fn policy_request() -> Result<CustomKeyPolicyRequest, DecodeError> {
    let existing = verification_token_from_row(&object(r#"{"key_alias":"old"}"#)).unwrap();
    let request = PolicyRequestBody::Update(update_request(r#"{"key_alias":"new"}"#));
    Ok(update_policy_request(
        PolicyOperation::Update,
        &existing,
        &object(r#"{"key_alias":"new"}"#),
        request,
    )
    .unwrap())
}

#[tokio::test]
async fn hook_failure_propagates() {
    let workspace = workspace();
    let service = PythonHooks::new(Some(workspace.config));
    let data = update_request(r#"{"key_alias":"alias"}"#);
    let broken = load(&service, HookSlot::CustomKeyUpdate, "update_broken").await;
    let error = enforce_key_update_hook(Some(&broken), &data)
        .await
        .unwrap_err();
    assert!(matches!(&error, HookPolicyError::Call(_)));
    assert!(error.to_string().contains("boom"), "{error}");
    let bad_decision = load(&service, HookSlot::CustomKeyUpdate, "update_bad_decision").await;
    assert!(matches!(
        enforce_key_update_hook(Some(&bad_decision), &data)
            .await
            .unwrap_err(),
        HookPolicyError::DecisionNotBool { field: "decision" }
    ));
    let bad_message = load(
        &service,
        HookSlot::CustomKeyUpdate,
        "update_deny_bad_message",
    )
    .await;
    assert!(matches!(
        enforce_key_update_hook(Some(&bad_message), &data)
            .await
            .unwrap_err(),
        HookPolicyError::MessageNotString { field: "message" }
    ));
    let not_mapping = load(&service, HookSlot::CustomKeyUpdate, "update_not_mapping").await;
    assert!(matches!(
        enforce_key_update_hook(Some(&not_mapping), &data)
            .await
            .unwrap_err(),
        HookPolicyError::ResultNotAnObject
    ));
    service.shutdown().await;
}
