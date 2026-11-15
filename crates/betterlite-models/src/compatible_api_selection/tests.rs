#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use rstest::rstest;

use super::{
    CompatibleApiPolicy, MESSAGES_ENDPOINT, MessagesApiPolicy, ProviderIdentity,
    RESPONSES_ENDPOINT, ResponsesApiPolicy, embedding_api_policy, image_generation_api_policy,
    messages_api_policy, rerank_api_policy, responses_api_policy,
};
use crate::identity::provider_id::ProviderId;
use crate::json_provider_registry::{
    BaseClass, Constraints, JSONProviderRegistry, ProviderConfig, ProviderDescriptor,
};

fn descriptor(provider: ProviderId, endpoints: &[&str]) -> ProviderDescriptor {
    ProviderDescriptor {
        provider,
        config: ProviderConfig {
            base_url: String::new(),
            api_key_env: String::new(),
            api_base_env: None,
            base_class: BaseClass::default(),
            param_mappings: BTreeMap::new(),
            constraints: Constraints::default(),
            special_handling: BTreeMap::new(),
            supported_endpoints: endpoints
                .iter()
                .map(|endpoint| (*endpoint).to_owned())
                .collect(),
        },
    }
}

fn registry(entries: &[(ProviderId, &[&str])]) -> JSONProviderRegistry {
    let providers: IndexMap<ProviderId, ProviderDescriptor> = entries
        .iter()
        .map(|(provider, endpoints)| (*provider, descriptor(*provider, endpoints)))
        .collect();
    JSONProviderRegistry {
        providers,
        loaded: true,
    }
}

#[test]
fn responses_native_policy_wins_over_a_json_collision() {
    let registry = registry(&[(ProviderId::Openai, &[RESPONSES_ENDPOINT])]);
    let policy = responses_api_policy(
        ProviderIdentity::Name("openai"),
        |known| known.map(|_| ResponsesApiPolicy::Native),
        &registry,
    );
    assert_eq!(policy, Some(ResponsesApiPolicy::Native));
}

#[rstest]
#[case::exact(&["/v1/chat/completions", "/v1/responses"], Some(ResponsesApiPolicy::Json(ProviderId::Groq)))]
#[case::suffix(&["/v1/responses/v2"], None)]
#[case::prefix(&["x/v1/responses"], None)]
#[case::empty(&[], None)]
fn responses_uses_a_json_descriptor_only_for_the_exact_endpoint(
    #[case] endpoints: &[&str],
    #[case] expected: Option<ResponsesApiPolicy>,
) {
    let registry = registry(&[(ProviderId::Groq, endpoints)]);
    let policy = responses_api_policy(ProviderIdentity::Name("groq"), |_| None, &registry);
    assert_eq!(policy, expected);
}

#[rstest]
#[case::enum_identity(ProviderIdentity::Enum(ProviderId::Openai))]
#[case::wire_name(ProviderIdentity::Name("openai"))]
fn responses_enum_and_name_identities_ask_the_native_selector_with_the_member(
    #[case] identity: ProviderIdentity<'static>,
) {
    let mut asked = None;
    let policy = responses_api_policy(
        identity,
        |known| {
            asked = Some(known);
            Some(ResponsesApiPolicy::Native)
        },
        &registry(&[]),
    );
    assert_eq!(asked, Some(Some(ProviderId::Openai)));
    assert_eq!(policy, Some(ResponsesApiPolicy::Native));
}

#[test]
fn responses_an_unknown_name_asks_without_a_member_and_still_reaches_the_registry() {
    let registry = registry(&[(ProviderId::Groq, &[RESPONSES_ENDPOINT])]);
    let mut asked = None;
    let policy = responses_api_policy(
        ProviderIdentity::Name("not_a_provider"),
        |known| {
            asked = Some(known);
            None
        },
        &registry,
    );
    assert_eq!(asked, Some(None));
    assert_eq!(policy, None);
}

#[test]
fn messages_deepseek_is_native_regardless_of_registry_contents() {
    let registry = registry(&[(ProviderId::Deepseek, &[MESSAGES_ENDPOINT])]);
    let policy = messages_api_policy(ProviderId::Deepseek, &registry);
    assert_eq!(policy, Some(MessagesApiPolicy::Deepseek));
    assert_eq!(
        MessagesApiPolicy::Deepseek.class_name(),
        "DeepSeekAnthropicMessagesConfig"
    );
}

#[rstest]
#[case::exact(&["/v1/messages"], Some(MessagesApiPolicy::Json(ProviderId::Groq)))]
#[case::suffix(&["/v1/messages/v2"], None)]
#[case::prefix(&["v1/messages"], None)]
#[case::empty(&[], None)]
fn messages_json_requires_exact_endpoint_membership(
    #[case] endpoints: &[&str],
    #[case] expected: Option<MessagesApiPolicy>,
) {
    let registry = registry(&[(ProviderId::Groq, endpoints)]);
    assert_eq!(messages_api_policy(ProviderId::Groq, &registry), expected);
}

#[test]
fn messages_json_reports_the_json_class_name() {
    let registry = registry(&[(ProviderId::Groq, &[MESSAGES_ENDPOINT])]);
    let policy = messages_api_policy(ProviderId::Groq, &registry).unwrap();
    assert_eq!(policy.class_name(), "JSONProviderAnthropicMessagesConfig");
}

#[rstest]
#[case::cometapi(ProviderId::Cometapi, Some(CompatibleApiPolicy::CometapiEmbedding))]
#[case::openai(ProviderId::Openai, None)]
#[case::deepinfra(ProviderId::Deepinfra, None)]
fn embedding_policy_selects_only_cometapi(
    #[case] provider: ProviderId,
    #[case] expected: Option<CompatibleApiPolicy>,
) {
    assert_eq!(embedding_api_policy(provider), expected);
}

#[rstest]
#[case::deepinfra(ProviderId::Deepinfra, Some(CompatibleApiPolicy::DeepinfraRerank))]
#[case::cometapi(ProviderId::Cometapi, None)]
#[case::cohere(ProviderId::Cohere, None)]
fn rerank_policy_selects_only_deepinfra(
    #[case] provider: ProviderId,
    #[case] expected: Option<CompatibleApiPolicy>,
) {
    assert_eq!(rerank_api_policy(provider), expected);
}

#[rstest]
#[case::aiml(ProviderId::Aiml, Some(CompatibleApiPolicy::AimlImageGeneration))]
#[case::cometapi(
    ProviderId::Cometapi,
    Some(CompatibleApiPolicy::CometapiImageGeneration)
)]
#[case::openai(ProviderId::Openai, None)]
fn image_generation_policy_selects_both_factories(
    #[case] provider: ProviderId,
    #[case] expected: Option<CompatibleApiPolicy>,
) {
    assert_eq!(image_generation_api_policy(provider), expected);
}

#[test]
fn compatible_policies_report_each_config_class_name() {
    assert_eq!(
        CompatibleApiPolicy::CometapiEmbedding.class_name(),
        "CometAPIEmbeddingConfig"
    );
    assert_eq!(
        CompatibleApiPolicy::DeepinfraRerank.class_name(),
        "DeepinfraRerankConfig"
    );
    assert_eq!(
        CompatibleApiPolicy::AimlImageGeneration.class_name(),
        "AimlImageGenerationConfig"
    );
    assert_eq!(
        CompatibleApiPolicy::CometapiImageGeneration.class_name(),
        "CometAPIImageGenerationConfig"
    );
}
