#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use super::{
    EmbeddingConfig, RerankConfig, embedding_config, rerank_config, should_use_cohere_v1_client,
};

fn params(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[rstest]
#[case::cohere(
    ProviderId::Cohere,
    "embed-english-v3.0",
    Some(EmbeddingConfig::Cohere)
)]
#[case::cohere_chat(ProviderId::CohereChat, "command-a", Some(EmbeddingConfig::Cohere))]
#[case::voyage_text(ProviderId::Voyage, "voyage-3-large", Some(EmbeddingConfig::Voyage))]
#[case::voyage_contextual(
    ProviderId::Voyage,
    "voyage-context-3",
    Some(EmbeddingConfig::VoyageContextual)
)]
#[case::voyage_context_case(
    ProviderId::Voyage,
    "Voyage-Context-3",
    Some(EmbeddingConfig::VoyageContextual)
)]
#[case::voyage_multimodal(
    ProviderId::Voyage,
    "voyage-multimodal-3",
    Some(EmbeddingConfig::VoyageMultimodal)
)]
#[case::voyage_context_before_multimodal(
    ProviderId::Voyage,
    "voyage-context-multimodal",
    Some(EmbeddingConfig::VoyageContextual)
)]
#[case::jina(
    ProviderId::JinaAi,
    "jina-embeddings-v4",
    Some(EmbeddingConfig::JinaAi)
)]
#[case::infinity(ProviderId::Infinity, "bge-base", Some(EmbeddingConfig::Infinity))]
#[case::openai(ProviderId::Openai, "text-embedding-3-small", None)]
#[case::huggingface(ProviderId::Huggingface, "voyage-context-3", None)]
fn the_family_embedding_arms_select_their_config(
    #[case] provider: ProviderId,
    #[case] model: &str,
    #[case] expected: Option<EmbeddingConfig>,
) {
    assert_eq!(embedding_config(provider, model), expected);
}

#[rstest]
#[case::cohere(EmbeddingConfig::Cohere, "CohereEmbeddingConfig")]
#[case::voyage(EmbeddingConfig::Voyage, "VoyageEmbeddingConfig")]
#[case::voyage_contextual(EmbeddingConfig::VoyageContextual, "VoyageContextualEmbeddingConfig")]
#[case::voyage_multimodal(EmbeddingConfig::VoyageMultimodal, "VoyageMultimodalEmbeddingConfig")]
#[case::jina(EmbeddingConfig::JinaAi, "JinaAIEmbeddingConfig")]
#[case::infinity(EmbeddingConfig::Infinity, "InfinityEmbeddingConfig")]
fn the_embedding_config_names_are_litellms_class_names(
    #[case] config: EmbeddingConfig,
    #[case] expected: &str,
) {
    assert_eq!(config.to_string(), expected);
}

#[rstest]
#[case::cohere(ProviderId::Cohere, None, &[], Some(RerankConfig::CohereV2))]
#[case::cohere_chat(ProviderId::CohereChat, Some("https://gateway.test/cohere"), &[], Some(RerankConfig::CohereV2))]
#[case::v1_suffix(ProviderId::Cohere, Some("https://api.cohere.ai/v1/rerank"), &[], Some(RerankConfig::Cohere))]
#[case::v1_suffix_with_legacy_params(ProviderId::Cohere, Some("https://api.cohere.ai/v1/rerank"), &["max_chunks_per_doc"], Some(RerankConfig::Cohere))]
#[case::v2_suffix_alone(ProviderId::Cohere, Some("https://api.cohere.ai/v2/rerank"), &[], Some(RerankConfig::CohereV2))]
#[case::legacy_params_on_another_base(ProviderId::Cohere, Some("https://proxy.test"), &["max_chunks_per_doc"], Some(RerankConfig::Cohere))]
#[case::legacy_params_with_the_v2_suffix(ProviderId::Cohere, Some("https://proxy.test/v2/rerank"), &["max_chunks_per_doc"], Some(RerankConfig::CohereV2))]
#[case::no_base_with_legacy_params(ProviderId::Cohere, None, &["max_chunks_per_doc"], Some(RerankConfig::CohereV2))]
#[case::empty_base_with_legacy_params(ProviderId::Cohere, Some(""), &["max_chunks_per_doc"], Some(RerankConfig::CohereV2))]
#[case::infinity(ProviderId::Infinity, None, &[], Some(RerankConfig::Infinity))]
#[case::jina(ProviderId::JinaAi, None, &[], Some(RerankConfig::JinaAi))]
#[case::voyage(ProviderId::Voyage, None, &[], Some(RerankConfig::Voyage))]
#[case::openai(ProviderId::Openai, None, &[], None)]
fn the_family_rerank_arms_select_their_config(
    #[case] provider: ProviderId,
    #[case] api_base: Option<&str>,
    #[case] present: &[&str],
    #[case] expected: Option<RerankConfig>,
) {
    assert_eq!(
        rerank_config(provider, api_base, &params(present)),
        expected
    );
}

#[rstest]
#[case::v1_suffix(Some("https://api.cohere.ai/v1/rerank"), &[], true)]
#[case::v1_suffix_with_v2_params(Some("https://api.cohere.ai/v1/rerank"), &["max_tokens_per_doc"], true)]
#[case::v2_suffix(Some("https://api.cohere.ai/v2/rerank"), &[], false)]
#[case::legacy_params_alone(Some("https://proxy.test"), &["max_chunks_per_doc"], true)]
#[case::legacy_params_with_v2_suffix(Some("https://proxy.test/v2/rerank"), &["max_chunks_per_doc"], false)]
#[case::both_limit_params(Some("https://proxy.test"), &["max_chunks_per_doc", "max_tokens_per_doc"], false)]
#[case::only_the_v2_param(Some("https://proxy.test"), &["max_tokens_per_doc"], false)]
#[case::other_params(Some("https://proxy.test"), &["top_n", "return_documents"], false)]
#[case::no_base(None, &["max_chunks_per_doc"], false)]
#[case::empty_base(Some(""), &["max_chunks_per_doc"], false)]
#[case::a_trailing_slash_breaks_the_v1_suffix(Some("https://api.cohere.ai/v1/rerank/"), &[], false)]
#[case::a_trailing_slash_breaks_the_v2_suffix(Some("https://proxy.test/v2/rerank/"), &["max_chunks_per_doc"], true)]
fn the_v1_client_choice_follows_the_base_and_the_legacy_params(
    #[case] api_base: Option<&str>,
    #[case] present: &[&str],
    #[case] expected: bool,
) {
    assert_eq!(
        should_use_cohere_v1_client(api_base, &params(present)),
        expected
    );
}

#[rstest]
#[case::cohere(RerankConfig::Cohere, "CohereRerankConfig")]
#[case::cohere_v2(RerankConfig::CohereV2, "CohereRerankV2Config")]
#[case::infinity(RerankConfig::Infinity, "InfinityRerankConfig")]
#[case::jina(RerankConfig::JinaAi, "JinaAIRerankConfig")]
#[case::voyage(RerankConfig::Voyage, "VoyageRerankConfig")]
fn the_rerank_config_names_are_litellms_class_names(
    #[case] config: RerankConfig,
    #[case] expected: &str,
) {
    assert_eq!(config.to_string(), expected);
}
