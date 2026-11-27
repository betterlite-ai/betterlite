#![cfg(test)]

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_core::api::vector_store_read::service::VectorStoreReadConfigs as _;
use betterlite_core::api::vector_store_search::provider::ResolvedProvider;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::{ExposeSecret, SecretString};

use super::{Configs, globals};

fn resolved(provider: ProviderId, tag: VectorStoresConfigTag) -> ResolvedProvider {
    ResolvedProvider {
        provider,
        api_type: None,
        tag,
    }
}

#[test]
fn configs_serve_openai_and_refuse_the_other_tags() {
    let credentials = OpenAiCredentialsSettings::default();
    let module_globals = LiteLlmGlobals::default();
    let env_api_key: Option<SecretString> = None;
    let configs = Configs {
        credentials: &credentials,
        globals: &module_globals,
        env_api_key: &env_api_key,
    };
    let openai = configs.read_config(&resolved(ProviderId::Openai, VectorStoresConfigTag::OpenAI));
    let azure = configs.read_config(&resolved(
        ProviderId::Azure,
        VectorStoresConfigTag::AzureOpenAI,
    ));
    assert!(openai.is_some());
    assert!(azure.is_none());
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
