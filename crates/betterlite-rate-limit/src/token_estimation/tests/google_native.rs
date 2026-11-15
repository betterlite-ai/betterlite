#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use betterlite_core::api::genai_chat_input::TranslationContext;
use betterlite_core::schema::SchemaLimits;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use rstest::rstest;

use super::google::translate_google_request;
use super::services::TranslatedChat;
use super::support::{RecordingTranslation, chat_reply, request, value};

fn real_translation(body: &str) -> Option<TranslatedChat> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let capabilities = CatalogCapabilities {
        chat_models: BTreeSet::from([String::from("m")]),
        text_models: BTreeSet::new(),
    };
    let context = TranslationContext {
        resolution: &resolution,
        model_capabilities: &capabilities,
        limits: SchemaLimits::default(),
    };
    translate_google_request(&context, &request(body), Some("generate_content"))
}

fn recorder() -> RecordingTranslation {
    RecordingTranslation {
        reply: Some(chat_reply(r#"[{"role":"user","content":"translated"}]"#)),
        seen: Mutex::new(Vec::new()),
    }
}

fn first_seen(fake: &RecordingTranslation) -> super::support::SeenTranslation {
    let seen = fake.seen.lock().expect("the translation recording lock");
    seen[0].clone()
}

#[test]
fn the_real_adapter_translates_native_contents_to_messages() {
    let translated = real_translation(r#"{"model":"m","contents":[{"parts":[{"text":"hi"}]}]}"#)
        .expect("the translated request");
    assert_eq!(
        translated.messages,
        value(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[test]
fn the_adapter_result_is_returned() {
    let fake = recorder();
    let translated = translate_google_request(
        &fake,
        &request(r#"{"model":"m","contents":[]}"#),
        Some("generate_content"),
    );
    assert_eq!(
        translated,
        Some(chat_reply(r#"[{"role":"user","content":"translated"}]"#))
    );
}

#[test]
fn a_single_content_object_is_translated() {
    let fake = recorder();
    let translated = translate_google_request(
        &fake,
        &request(r#"{"model":"m","contents":{"parts":[{"text":"hi"}]}}"#),
        Some("generate_content"),
    );
    assert_eq!(
        translated,
        Some(chat_reply(r#"[{"role":"user","content":"translated"}]"#))
    );
    assert_eq!(
        first_seen(&fake).contents,
        value(r#"{"parts":[{"text":"hi"}]}"#)
    );
}

#[test]
fn a_config_field_wins_over_the_generation_config() {
    let fake = recorder();
    let _translated = translate_google_request(
        &fake,
        &request(
            r#"{"model":"m","contents":[],"config":{"maxOutputTokens":7},"generationConfig":{"maxOutputTokens":9}}"#,
        ),
        Some("generate_content"),
    );
    assert_eq!(
        first_seen(&fake).config,
        Some(value(r#"{"maxOutputTokens":7}"#))
    );
}

#[test]
fn a_generation_config_is_used_when_no_config_is_present() {
    let fake = recorder();
    let _translated = translate_google_request(
        &fake,
        &request(r#"{"model":"m","contents":[],"generationConfig":{"temperature":1}}"#),
        Some("agenerate_content"),
    );
    assert_eq!(
        first_seen(&fake).config,
        Some(value(r#"{"temperature":1}"#))
    );
}

#[test]
fn a_non_mapping_config_is_not_forwarded() {
    let fake = recorder();
    let _translated = translate_google_request(
        &fake,
        &request(r#"{"model":"m","contents":[],"config":"none"}"#),
        Some("generate_content"),
    );
    assert_eq!(first_seen(&fake).config, None);
}

#[test]
fn a_missing_model_is_forwarded_as_empty_text() {
    let fake = recorder();
    let _translated = translate_google_request(
        &fake,
        &request(r#"{"contents":[],"model":5}"#),
        Some("generate_content"),
    );
    assert_eq!(first_seen(&fake).model, "");
}

#[test]
fn both_system_instruction_spellings_are_forwarded() {
    let fake = recorder();
    let _translated = translate_google_request(
        &fake,
        &request(
            r#"{"model":"m","contents":[],"systemInstruction":{"parts":[{"text":"a"}]},"system_instruction":{"parts":[{"text":"b"}]}}"#,
        ),
        Some("generate_content_stream"),
    );
    let seen = first_seen(&fake);
    assert_eq!(
        seen.system_instruction,
        Some(value(r#"{"parts":[{"text":"a"}]}"#))
    );
    assert_eq!(
        seen.system_instruction_snake,
        Some(value(r#"{"parts":[{"text":"b"}]}"#))
    );
}

#[test]
fn tools_and_tool_configs_are_forwarded() {
    let fake = recorder();
    let _translated = translate_google_request(
        &fake,
        &request(
            r#"{"model":"m","contents":[],"tools":[{"functionDeclarations":[]}],"toolConfig":{"functionCallingConfig":{"mode":"AUTO"}},"tool_config":{"functionCallingConfig":{"mode":"NONE"}}}"#,
        ),
        Some("generate_content"),
    );
    let seen = first_seen(&fake);
    assert_eq!(seen.tools, Some(value(r#"[{"functionDeclarations":[]}]"#)));
    assert_eq!(
        seen.tool_config,
        Some(value(r#"{"functionCallingConfig":{"mode":"AUTO"}}"#))
    );
    assert_eq!(
        seen.tool_config_snake,
        Some(value(r#"{"functionCallingConfig":{"mode":"NONE"}}"#))
    );
}

#[rstest]
#[case::text_contents(r#"{"model":"m","contents":"hi"}"#, Some("generate_content"))]
#[case::absent_contents(r#"{"model":"m"}"#, Some("generate_content"))]
#[case::null_contents(r#"{"model":"m","contents":null}"#, Some("generate_content"))]
#[case::other_call_type(r#"{"model":"m","contents":[]}"#, Some("completion"))]
#[case::no_call_type(r#"{"model":"m","contents":[]}"#, None)]
fn non_google_requests_are_not_translated(#[case] body: &str, #[case] call_type: Option<&str>) {
    let fake = recorder();
    let translated = translate_google_request(&fake, &request(body), call_type);
    assert_eq!(translated, None);
    assert!(
        fake.seen
            .lock()
            .expect("the translation recording lock")
            .is_empty()
    );
}
