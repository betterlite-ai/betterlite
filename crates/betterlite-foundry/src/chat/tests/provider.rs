use std::collections::BTreeSet;

use betterlite_models::identity::provider_id::ProviderId;
use secrecy::{ExposeSecret as _, SecretString};

use crate::chat::provider::{
    OpenAiModelLists, ProviderRequest, is_azure_openai_model, provider_info,
};

use super::support::model_set;

const AZURE_HOST: &str = "https://my.openai.azure.com";
const FOUNDRY_HOST: &str = "https://litellm8397336933.services.ai.azure.com";

struct Lists {
    chat: BTreeSet<String>,
    text: BTreeSet<String>,
    embedding: BTreeSet<String>,
}

impl Lists {
    fn borrowed(&self) -> OpenAiModelLists<'_> {
        OpenAiModelLists {
            chat: &self.chat,
            text: &self.text,
            embedding: &self.embedding,
        }
    }
}

fn lists(chat: &[&str], text: &[&str], embedding: &[&str]) -> Lists {
    Lists {
        chat: model_set(chat),
        text: model_set(text),
        embedding: model_set(embedding),
    }
}

fn catalog() -> Lists {
    lists(
        &["gpt-4o", "gpt-5"],
        &["text-davinci-003"],
        &["text-embedding-3-small"],
    )
}

pub(super) fn azure_openai_model_redirect() {
    let all = catalog();
    let deployment_path = "https://litellm8397336933.services.ai.azure.com/openai/deployments/d";
    let cases = [
        ("gpt-4o", Some(AZURE_HOST), true),
        ("openai/gpt-4o", Some(AZURE_HOST), true),
        // only the first prefix is removed, so a second one hides the catalog name
        ("openai/azure_ai/gpt-4o", Some(AZURE_HOST), false),
        ("text-davinci-003", Some(AZURE_HOST), true),
        ("text-embedding-3-small", Some(AZURE_HOST), true),
        ("unknown-model", Some(AZURE_HOST), false),
        // a Foundry model-inference host never redirects, while a deployment path on it does
        ("gpt-4o", Some(FOUNDRY_HOST), false),
        ("gpt-4o", Some(deployment_path), true),
        ("gpt-4o", None, false),
    ];
    for (model, base, expected) in cases {
        assert_eq!(
            is_azure_openai_model(model, base, all.borrowed()),
            expected,
            "{model} on {base:?}"
        );
    }
    let empty = lists(&[], &[], &[]);
    assert!(!is_azure_openai_model(
        "gpt-4o",
        Some(AZURE_HOST),
        empty.borrowed()
    ));
}

fn info<'data>(
    model: &str,
    api_base: Option<&'data str>,
    api_key: Option<&'data SecretString>,
    environment: (Option<&'data str>, Option<&'data SecretString>),
    lists: OpenAiModelLists<'data>,
) -> (Option<String>, Option<String>, ProviderId) {
    let resolved = provider_info(&ProviderRequest {
        model,
        api_base,
        api_key,
        custom_llm_provider: ProviderId::AzureAi,
        environment_base: environment.0,
        environment_key: environment.1,
        lists,
    });
    (
        resolved.api_base,
        resolved
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        resolved.custom_llm_provider,
    )
}

#[test]
fn provider_info_prefers_explicit_values_then_the_environment() {
    let all = catalog();
    let key = SecretString::from("explicit-key");
    let env_key = SecretString::from("env-key");
    let environment = (Some("https://env.openai.azure.com"), Some(&env_key));
    assert_eq!(
        info(
            "gpt-4o",
            Some("https://explicit.example"),
            Some(&key),
            environment,
            all.borrowed()
        ),
        (
            Some("https://explicit.example".to_owned()),
            Some("explicit-key".to_owned()),
            ProviderId::Azure
        )
    );
    assert_eq!(
        info("gpt-4o", None, None, environment, all.borrowed()),
        (
            Some("https://env.openai.azure.com".to_owned()),
            Some("env-key".to_owned()),
            ProviderId::Azure
        )
    );
    // no base anywhere means no redirect, and the provider is echoed back
    let empty = (None, None);
    assert_eq!(
        info("gpt-4o", None, None, empty, all.borrowed()),
        (None, None, ProviderId::AzureAi)
    );
}

#[test]
fn provider_info_never_redirects_a_foundry_base() {
    let all = catalog();
    let empty = (None, None);
    assert_eq!(
        info("gpt-4o", Some(FOUNDRY_HOST), None, empty, all.borrowed()),
        (Some(FOUNDRY_HOST.to_owned()), None, ProviderId::AzureAi)
    );
}
