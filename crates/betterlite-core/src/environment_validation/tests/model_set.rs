use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use rstest::rstest;

use super::{check_without_provider, missing, sets_with};

#[rstest]
#[case::openai_chat(ProviderModelSet::OpenAiChatCompletion, &["OPENAI_API_KEY"])]
#[case::openai_text(ProviderModelSet::OpenAiTextCompletion, &["OPENAI_API_KEY"])]
#[case::anthropic(ProviderModelSet::Anthropic, &["ANTHROPIC_API_KEY"])]
#[case::cohere(ProviderModelSet::Cohere, &["COHERE_API_KEY"])]
#[case::replicate(ProviderModelSet::Replicate, &["REPLICATE_API_KEY"])]
#[case::openrouter(ProviderModelSet::Openrouter, &["OPENROUTER_API_KEY"])]
#[case::vercel_ai_gateway(ProviderModelSet::VercelAiGateway, &["VERCEL_AI_GATEWAY_API_KEY"])]
#[case::edenai(ProviderModelSet::Edenai, &["EDENAI_API_KEY"])]
#[case::datarobot(ProviderModelSet::Datarobot, &["DATAROBOT_API_TOKEN"])]
#[case::vertex_chat(ProviderModelSet::VertexChat, &["VERTEXAI_PROJECT", "VERTEXAI_LOCATION"])]
#[case::vertex_text(ProviderModelSet::VertexText, &["VERTEXAI_PROJECT", "VERTEXAI_LOCATION"])]
#[case::huggingface(ProviderModelSet::Huggingface, &["HUGGINGFACE_API_KEY"])]
#[case::ai21(ProviderModelSet::Ai21, &["AI21_API_KEY"])]
#[case::together_ai(ProviderModelSet::TogetherAi, &["TOGETHERAI_API_KEY"])]
#[case::aleph_alpha(ProviderModelSet::AlephAlpha, &["ALEPH_ALPHA_API_KEY"])]
#[case::baseten(ProviderModelSet::Baseten, &["BASETEN_API_KEY"])]
#[case::nlp_cloud(ProviderModelSet::NlpCloud, &["NLP_CLOUD_API_KEY"])]
#[case::novita(ProviderModelSet::Novita, &["NOVITA_API_KEY"])]
#[case::nebius(ProviderModelSet::Nebius, &["NEBIUS_API_KEY"])]
#[case::wandb(ProviderModelSet::Wandb, &["WANDB_API_KEY"])]
fn model_set_row_names_its_provider_row(
    #[case] set: ProviderModelSet,
    #[case] required: &[&'static str],
) {
    let sets = sets_with(&[(set, "listed-model")]);
    assert_eq!(
        check_without_provider(Some("listed-model"), &sets, &[]),
        missing(required)
    );
    assert_eq!(
        check_without_provider(Some("listed-model"), &sets, required),
        missing(&[])
    );
}

#[rstest]
#[case::embedding_ada("text-embedding-ada-002")]
#[case::dall_e_2("dall-e-2")]
#[case::dall_e_3("dall-e-3")]
#[case::gpt_image("gpt-image")]
#[case::gpt_image_version("gpt-image-1")]
fn model_set_openai_fixed_names_and_image_prefix(#[case] model: &str) {
    let answer = check_without_provider(Some(model), &ProviderModelSets::default(), &[]);
    assert_eq!(answer, missing(&["OPENAI_API_KEY"]));
}

#[rstest]
#[case::near_embedding("text-embedding-ada-001")]
#[case::near_dall_e("dall-e-4")]
#[case::short_prefix("gpt-imag")]
#[case::prefix_not_at_start("my-gpt-image")]
#[case::empty("")]
fn model_set_unlisted_model_gives_true_and_nothing_missing(#[case] model: &str) {
    let answer = check_without_provider(Some(model), &ProviderModelSets::default(), &[]);
    assert_eq!(answer, missing(&[]));
}

#[test]
fn model_set_vertex_models_by_provider_name_the_vertex_row() {
    let mut sets = ProviderModelSets::default();
    sets.models_by_provider
        .entry("vertex_ai".to_owned())
        .or_default()
        .insert("vertex-only-model".to_owned());
    let answer = check_without_provider(Some("vertex-only-model"), &sets, &[]);
    assert_eq!(answer, missing(&["VERTEXAI_PROJECT", "VERTEXAI_LOCATION"]));
}

#[test]
fn model_set_other_provider_in_models_by_provider_is_not_consulted() {
    let mut sets = ProviderModelSets::default();
    sets.models_by_provider
        .entry("groq".to_owned())
        .or_default()
        .insert("groq-model".to_owned());
    assert_eq!(
        check_without_provider(Some("groq-model"), &sets, &[]),
        missing(&[])
    );
}

#[test]
fn model_set_no_model_gives_false_and_an_empty_list() {
    let sets = sets_with(&[(ProviderModelSet::Anthropic, "listed-model")]);
    let answer = check_without_provider(None, &sets, &[]);
    assert!(!answer.keys_in_environment);
    assert!(answer.missing_keys.is_empty());
}

#[rstest]
#[case::openai_over_anthropic(
    ProviderModelSet::OpenAiChatCompletion,
    ProviderModelSet::Anthropic,
    "OPENAI_API_KEY"
)]
#[case::openai_text_over_cohere(
    ProviderModelSet::OpenAiTextCompletion,
    ProviderModelSet::Cohere,
    "OPENAI_API_KEY"
)]
#[case::anthropic_over_cohere(
    ProviderModelSet::Anthropic,
    ProviderModelSet::Cohere,
    "ANTHROPIC_API_KEY"
)]
#[case::cohere_over_replicate(
    ProviderModelSet::Cohere,
    ProviderModelSet::Replicate,
    "COHERE_API_KEY"
)]
#[case::replicate_over_openrouter(
    ProviderModelSet::Replicate,
    ProviderModelSet::Openrouter,
    "REPLICATE_API_KEY"
)]
#[case::openrouter_over_vercel(
    ProviderModelSet::Openrouter,
    ProviderModelSet::VercelAiGateway,
    "OPENROUTER_API_KEY"
)]
#[case::vercel_over_edenai(
    ProviderModelSet::VercelAiGateway,
    ProviderModelSet::Edenai,
    "VERCEL_AI_GATEWAY_API_KEY"
)]
#[case::edenai_over_datarobot(
    ProviderModelSet::Edenai,
    ProviderModelSet::Datarobot,
    "EDENAI_API_KEY"
)]
#[case::datarobot_over_vertex(
    ProviderModelSet::Datarobot,
    ProviderModelSet::VertexChat,
    "DATAROBOT_API_TOKEN"
)]
#[case::vertex_over_huggingface(
    ProviderModelSet::VertexText,
    ProviderModelSet::Huggingface,
    "VERTEXAI_PROJECT"
)]
#[case::huggingface_over_ai21(
    ProviderModelSet::Huggingface,
    ProviderModelSet::Ai21,
    "HUGGINGFACE_API_KEY"
)]
#[case::ai21_over_together(ProviderModelSet::Ai21, ProviderModelSet::TogetherAi, "AI21_API_KEY")]
#[case::together_over_aleph(
    ProviderModelSet::TogetherAi,
    ProviderModelSet::AlephAlpha,
    "TOGETHERAI_API_KEY"
)]
#[case::aleph_over_baseten(
    ProviderModelSet::AlephAlpha,
    ProviderModelSet::Baseten,
    "ALEPH_ALPHA_API_KEY"
)]
#[case::baseten_over_nlp_cloud(
    ProviderModelSet::Baseten,
    ProviderModelSet::NlpCloud,
    "BASETEN_API_KEY"
)]
#[case::nlp_cloud_over_novita(
    ProviderModelSet::NlpCloud,
    ProviderModelSet::Novita,
    "NLP_CLOUD_API_KEY"
)]
#[case::novita_over_nebius(ProviderModelSet::Novita, ProviderModelSet::Nebius, "NOVITA_API_KEY")]
#[case::nebius_over_wandb(ProviderModelSet::Nebius, ProviderModelSet::Wandb, "NEBIUS_API_KEY")]
fn model_set_first_row_wins_when_a_model_is_in_two_sets(
    #[case] earlier: ProviderModelSet,
    #[case] later: ProviderModelSet,
    #[case] first_missing_name: &'static str,
) {
    let sets = sets_with(&[(later, "shared"), (earlier, "shared")]);
    let answer = check_without_provider(Some("shared"), &sets, &[]);
    assert_eq!(
        answer.missing_keys.first().copied(),
        Some(first_missing_name)
    );
}
