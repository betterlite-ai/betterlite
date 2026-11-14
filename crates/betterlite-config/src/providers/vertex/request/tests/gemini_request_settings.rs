#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_request::body::{BodyInput, transform};
use betterlite_vertex::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};
use tokio::runtime::Builder;

use crate::loader::{ConfigError, ConfigLoader};

const CACHED_MESSAGES: &str =
    r#"[{"role":"system","content":"rules"},{"role":"user","content":"hi"}]"#;
const CACHED_PARAMS: &str = r#"{"cached_content":"cached-1","tools":[{"googleSearch":{}}]}"#;

struct FixtureLookup;

impl ModelCapabilityLookup for FixtureLookup {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        Ok(match kind {
            CapabilityKind::SystemMessages | CapabilityKind::ResponseSchema => true,
        })
    }
}

struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<bool, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<String, String>>();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.litellm_settings.modify_params)
}

fn body(modify_params: bool) -> Object {
    let messages: Vec<Message> = serde_json::from_str(CACHED_MESSAGES).expect("fixture messages");
    let optional_params: Object = serde_json::from_str(CACHED_PARAMS).expect("fixture params");
    let litellm_params = Object::new();
    let input = BodyInput {
        provider: VertexCapabilityProvider::VertexAi,
        model: "gemini-2.5-pro",
        messages: &messages,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        cached_content: Some("cached-1"),
        modify_params,
        schema_template: None,
    };
    let runtime = Builder::new_current_thread()
        .build()
        .expect("a fixture runtime");
    runtime
        .block_on(transform(&input, &FixtureLookup, &NoFetch))
        .expect("the body builds")
        .body
}

#[test]
fn configured_modify_params_drops_cached_fields() {
    let modify_params =
        load("litellm_settings:\n  modify_params: true\n", &[]).expect("the configuration loads");
    assert!(modify_params);
    let built = body(modify_params);
    assert!(built.get("system_instruction").is_none());
    assert!(built.get("tools").is_none());
    assert!(built.contains_key("cachedContent"));
}

#[test]
fn the_default_keeps_cached_fields() {
    let modify_params = load("", &[]).expect("the configuration loads");
    assert!(!modify_params);
    let built = body(modify_params);
    assert!(built.contains_key("system_instruction"));
    assert!(built.contains_key("tools"));
    assert!(built.contains_key("cachedContent"));
}

#[test]
fn the_environment_fallback_enables_modify_params() {
    let modify_params =
        load("", &[("LITELLM_MODIFY_PARAMS", "1")]).expect("the configuration loads");
    assert!(modify_params);
    let built = body(modify_params);
    assert!(built.get("system_instruction").is_none());
    assert!(built.get("tools").is_none());
}

#[test]
fn configured_modify_params_beats_the_environment() {
    let modify_params = load(
        "litellm_settings:\n  modify_params: false\n",
        &[("LITELLM_MODIFY_PARAMS", "1")],
    )
    .expect("the configuration loads");
    assert!(!modify_params);
    let built = body(modify_params);
    assert!(built.contains_key("system_instruction"));
    assert!(built.contains_key("tools"));
}

#[test]
fn an_invalid_environment_value_names_the_variable() {
    let error =
        load("", &[("LITELLM_MODIFY_PARAMS", "maybe")]).expect_err("the value is not a boolean");
    assert!(
        error.to_string().contains("LITELLM_MODIFY_PARAMS"),
        "{error}"
    );
}
