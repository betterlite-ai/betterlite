#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_core::api::assistant_threads::error::ThreadError;
use pyo3_async_runtimes::tokio::get_runtime;
use secrecy::SecretString;

use super::{ModuleSettings, Session, ThreadKind, ThreadRequest, apply_module_settings};
use crate::assistant_calls::error::family::ThreadOutcome;

#[test]
fn module_settings_apply_every_global() {
    let module = ModuleSettings {
        api_key: Some(SecretString::from("sk-a")),
        api_base: Some(String::from("http://base")),
        api_version: Some(String::from("v1")),
        organization: Some(String::from("org")),
        openai_key: Some(SecretString::from("sk-b")),
        azure_key: Some(SecretString::from("az")),
    };
    let mut settings = Settings::default();
    apply_module_settings(&mut settings, &module);
    let key = settings
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let openai = settings
        .openai_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let azure = settings
        .azure_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    assert_eq!(key, Some("sk-a"));
    assert_eq!(settings.api_base.as_deref(), Some("http://base"));
    assert_eq!(settings.api_version.as_deref(), Some("v1"));
    assert_eq!(settings.organization.as_deref(), Some("org"));
    assert_eq!(openai, Some("sk-b"));
    assert_eq!(azure, Some("az"));
}

// a provider no thread call supports answers the mapped error instead of a thread
#[test]
fn thread_for_an_unsupported_provider_answers_an_error() {
    let session = Session::build("{}").unwrap();
    let request = ThreadRequest {
        custom_llm_provider: String::from("bedrock"),
        thread_id: None,
        messages: None,
        metadata: None,
        tool_resources: None,
        kwargs: BTreeMap::new(),
    };
    let outcome = get_runtime().block_on(session.thread(ThreadKind::Create, &request, false));
    assert!(matches!(
        outcome,
        Err(ThreadOutcome::Sync(ThreadError::Unsupported { .. }))
    ));
}
