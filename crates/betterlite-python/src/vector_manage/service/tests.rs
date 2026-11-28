#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_core::api::vector_store_read::service::VectorStoreReadConfigs as _;
use betterlite_core::api::vector_store_search::provider::ResolvedProvider;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret, SecretString};

use super::{SessionService, globals};
use crate::vector_manage::session::SystemClock;
use crate::vector_manage::source::ProxiedSource;

#[test]
fn configs_serve_openai_and_refuse_the_other_tags() {
    let credentials = OpenAiCredentialsSettings::default();
    let module_globals = LiteLlmGlobals::default();
    let env_api_key: Option<SecretString> = None;
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("{}")
    .expect("the empty config loads");
    let routing = Arc::new(
        ProxyPolicy::resolve(&config.litellm_settings.proxy_trust, &BTreeMap::new())
            .expect("the empty environment resolves"),
    );
    let source = ProxiedSource::new(config, BTreeMap::new(), routing);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds");
    let clock = SystemClock::new();
    let adapter = SessionService {
        credentials: &credentials,
        globals: &module_globals,
        env_api_key: &env_api_key,
        source: &source,
        redactor: &redactor,
        clock: &clock,
    };
    let openai = ResolvedProvider {
        provider: ProviderId::Openai,
        api_type: None,
        tag: VectorStoresConfigTag::OpenAI,
    };
    let azure = ResolvedProvider {
        provider: ProviderId::Azure,
        api_type: None,
        tag: VectorStoresConfigTag::AzureOpenAI,
    };
    assert!(adapter.read_config(&openai).is_some());
    assert!(adapter.read_config(&azure).is_none());
}

#[test]
fn globals_copies_every_credential_field() {
    let source = LiteLlmGlobals {
        api_base: Some(String::from("http://127.0.0.1:9/v1")),
        api_key: Some(SecretString::from("sk-key")),
        openai_key: None,
        organization: Some(String::from("team")),
    };
    let copied = globals(&source);
    assert_eq!(copied.api_base, source.api_base);
    assert_eq!(copied.organization, source.organization);
    assert_eq!(
        copied.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-key")
    );
    assert!(copied.openai_key.is_none());
}
