#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use crate::catalog::{ModelCatalog, UnknownFields};
use crate::metadata::identity::Mode;

use super::{ProviderModelSet, ProviderModelSets};

const BY_PROVIDER_KEYS: [&str; 101] = [
    "ai21",
    "aiml",
    "aleph_alpha",
    "amazon_nova",
    "anthropic",
    "anyscale",
    "assemblyai",
    "aws_polly",
    "azure",
    "azure_ai",
    "azure_anthropic",
    "azure_text",
    "baseten",
    "bedrock",
    "bedrock_mantle",
    "black_forest_labs",
    "cerebras",
    "chatgpt",
    "clarifai",
    "cloudflare",
    "codestral",
    "cohere",
    "cohere_chat",
    "cometapi",
    "darkbloom",
    "dashscope",
    "databricks",
    "datarobot",
    "deepgram",
    "deepinfra",
    "deepseek",
    "edenai",
    "elevenlabs",
    "fal_ai",
    "featherless_ai",
    "fireworks_ai",
    "friendliai",
    "galadriel",
    "gemini",
    "gigachat",
    "github_copilot",
    "gradient_ai",
    "groq",
    "heroku",
    "huggingface",
    "hyperbolic",
    "inception",
    "infinity",
    "jina_ai",
    "lambda_ai",
    "lemonade",
    "llamagate",
    "maritalk",
    "meta_llama",
    "minimax",
    "mistral",
    "modelscope",
    "moonshot",
    "morph",
    "nadir",
    "nebius",
    "nlp_cloud",
    "novita",
    "nscale",
    "nvidia_nim",
    "nvidia_riva",
    "oci",
    "ollama",
    "ollama_chat",
    "openai",
    "openrouter",
    "ovhcloud",
    "palm",
    "perplexity",
    "petals",
    "publicai",
    "qwen_ai_platform",
    "qwencloud",
    "recraft",
    "reducto",
    "replicate",
    "runwayml",
    "sambanova",
    "snowflake",
    "soniox",
    "stability",
    "tencent",
    "text-completion-codestral",
    "text-completion-inception",
    "text-completion-openai",
    "together_ai",
    "transcribe",
    "v0",
    "vercel_ai_gateway",
    "vertex_ai",
    "volcengine",
    "voyage",
    "wandb",
    "watsonx",
    "xai",
    "zai",
];

fn catalog_of(entries: &[(&str, &str)]) -> String {
    let body: Vec<String> = entries
        .iter()
        .map(|(key, provider)| format!(r#""{key}":{{"litellm_provider":"{provider}"}}"#))
        .collect();
    format!("{{{}}}", body.join(","))
}

fn fold(json: &str) -> ProviderModelSets {
    let catalog = ModelCatalog::parse(json.as_bytes(), UnknownFields::Warn).unwrap();
    let mut sets = ProviderModelSets::default();
    sets.add_known_models(&catalog);
    sets
}

fn names(sets: &ProviderModelSets, set: ProviderModelSet) -> Vec<String> {
    sets.sets[&set].iter().cloned().collect()
}

fn provider_names(sets: &ProviderModelSets, provider: &str) -> Vec<String> {
    sets.models_by_provider[provider].iter().cloned().collect()
}

fn json_with_mode(key: &str, provider: &str, mode: Option<Mode>) -> String {
    let Some(declared) = mode else {
        return catalog_of(&[(key, provider)]);
    };
    format!(
        r#"{{"{key}":{{"litellm_provider":"{provider}","mode":{}}}}}"#,
        serde_json::to_string(&declared).unwrap()
    )
}

#[rstest]
#[case::openai("openai", None, ProviderModelSet::OpenAiChatCompletion)]
#[case::text_completion_openai(
    "text-completion-openai",
    None,
    ProviderModelSet::OpenAiTextCompletion
)]
#[case::azure_text("azure_text", None, ProviderModelSet::AzureText)]
#[case::cohere("cohere", None, ProviderModelSet::Cohere)]
#[case::cohere_chat("cohere_chat", None, ProviderModelSet::CohereChat)]
#[case::mistral("mistral", None, ProviderModelSet::MistralChat)]
#[case::anthropic("anthropic", None, ProviderModelSet::Anthropic)]
#[case::empower("empower", None, ProviderModelSet::Empower)]
#[case::openrouter("openrouter", None, ProviderModelSet::Openrouter)]
#[case::vercel_ai_gateway("vercel_ai_gateway", None, ProviderModelSet::VercelAiGateway)]
#[case::edenai("edenai", None, ProviderModelSet::Edenai)]
#[case::datarobot("datarobot", None, ProviderModelSet::Datarobot)]
#[case::vertex_ai_text_models("vertex_ai-text-models", None, ProviderModelSet::VertexText)]
#[case::vertex_ai_code_text_models(
    "vertex_ai-code-text-models",
    None,
    ProviderModelSet::VertexCodeText
)]
#[case::vertex_ai_language_models(
    "vertex_ai-language-models",
    None,
    ProviderModelSet::VertexLanguage
)]
#[case::vertex_ai_vision_models("vertex_ai-vision-models", None, ProviderModelSet::VertexVision)]
#[case::vertex_ai_chat_models("vertex_ai-chat-models", None, ProviderModelSet::VertexChat)]
#[case::vertex_ai_code_chat_models(
    "vertex_ai-code-chat-models",
    None,
    ProviderModelSet::VertexCodeChat
)]
#[case::vertex_ai_embedding_models(
    "vertex_ai-embedding-models",
    None,
    ProviderModelSet::VertexEmbedding
)]
#[case::vertex_ai_anthropic_models(
    "vertex_ai-anthropic_models",
    None,
    ProviderModelSet::VertexAnthropic
)]
#[case::vertex_ai_llama_models("vertex_ai-llama_models", None, ProviderModelSet::VertexLlama3)]
#[case::vertex_ai_deepseek_models(
    "vertex_ai-deepseek_models",
    None,
    ProviderModelSet::VertexDeepseek
)]
#[case::vertex_ai_mistral_models("vertex_ai-mistral_models", None, ProviderModelSet::VertexMistral)]
#[case::vertex_ai_ai21_models("vertex_ai-ai21_models", None, ProviderModelSet::VertexAiAi21)]
#[case::vertex_ai_image_models("vertex_ai-image-models", None, ProviderModelSet::VertexAiImage)]
#[case::vertex_ai_video_models("vertex_ai-video-models", None, ProviderModelSet::VertexAiVideo)]
#[case::vertex_ai_openai_models("vertex_ai-openai_models", None, ProviderModelSet::VertexOpenai)]
#[case::vertex_ai_minimax_models("vertex_ai-minimax_models", None, ProviderModelSet::VertexMinimax)]
#[case::vertex_ai_moonshot_models(
    "vertex_ai-moonshot_models",
    None,
    ProviderModelSet::VertexMoonshot
)]
#[case::vertex_ai_zai_models("vertex_ai-zai_models", None, ProviderModelSet::VertexZai)]
#[case::nlp_cloud("nlp_cloud", None, ProviderModelSet::NlpCloud)]
#[case::aleph_alpha("aleph_alpha", None, ProviderModelSet::AlephAlpha)]
#[case::bedrock_converse("bedrock_converse", None, ProviderModelSet::BedrockConverse)]
#[case::deepinfra("deepinfra", None, ProviderModelSet::Deepinfra)]
#[case::perplexity("perplexity", None, ProviderModelSet::Perplexity)]
#[case::watsonx("watsonx", None, ProviderModelSet::Watsonx)]
#[case::gemini("gemini", None, ProviderModelSet::Gemini)]
#[case::text_completion_codestral(
    "text-completion-codestral",
    None,
    ProviderModelSet::TextCompletionCodestral
)]
#[case::text_completion_inception(
    "text-completion-inception",
    None,
    ProviderModelSet::TextCompletionInception
)]
#[case::xai("xai", None, ProviderModelSet::Xai)]
#[case::zai("zai", None, ProviderModelSet::Zai)]
#[case::fal_ai("fal_ai", None, ProviderModelSet::FalAi)]
#[case::deepseek("deepseek", None, ProviderModelSet::Deepseek)]
#[case::tencent("tencent", None, ProviderModelSet::Tencent)]
#[case::runwayml("runwayml", None, ProviderModelSet::Runwayml)]
#[case::meta_llama("meta_llama", None, ProviderModelSet::Llama)]
#[case::nscale("nscale", None, ProviderModelSet::Nscale)]
#[case::azure_ai("azure_ai", None, ProviderModelSet::AzureAi)]
#[case::voyage("voyage", None, ProviderModelSet::Voyage)]
#[case::infinity("infinity", None, ProviderModelSet::Infinity)]
#[case::databricks("databricks", None, ProviderModelSet::Databricks)]
#[case::cloudflare("cloudflare", None, ProviderModelSet::Cloudflare)]
#[case::codestral("codestral", None, ProviderModelSet::Codestral)]
#[case::friendliai("friendliai", None, ProviderModelSet::Friendliai)]
#[case::palm("palm", None, ProviderModelSet::Palm)]
#[case::groq("groq", None, ProviderModelSet::Groq)]
#[case::azure("azure", None, ProviderModelSet::Azure)]
#[case::azure_anthropic("azure_anthropic", None, ProviderModelSet::AzureAnthropic)]
#[case::anyscale("anyscale", None, ProviderModelSet::Anyscale)]
#[case::cerebras("cerebras", None, ProviderModelSet::Cerebras)]
#[case::nadir("nadir", None, ProviderModelSet::Nadir)]
#[case::galadriel("galadriel", None, ProviderModelSet::Galadriel)]
#[case::nvidia_nim("nvidia_nim", None, ProviderModelSet::NvidiaNim)]
#[case::nvidia_riva("nvidia_riva", None, ProviderModelSet::NvidiaRiva)]
#[case::soniox("soniox", None, ProviderModelSet::Soniox)]
#[case::sambanova("sambanova", None, ProviderModelSet::Sambanova)]
#[case::sambanova_embedding_models(
    "sambanova-embedding-models",
    None,
    ProviderModelSet::SambanovaEmbedding
)]
#[case::novita("novita", None, ProviderModelSet::Novita)]
#[case::nebius_chat_models("nebius-chat-models", None, ProviderModelSet::Nebius)]
#[case::nebius_embedding_models("nebius-embedding-models", None, ProviderModelSet::NebiusEmbedding)]
#[case::aiml("aiml", None, ProviderModelSet::Aiml)]
#[case::assemblyai("assemblyai", None, ProviderModelSet::Assemblyai)]
#[case::jina_ai("jina_ai", None, ProviderModelSet::JinaAi)]
#[case::snowflake("snowflake", None, ProviderModelSet::Snowflake)]
#[case::gradient_ai("gradient_ai", None, ProviderModelSet::GradientAi)]
#[case::featherless_ai("featherless_ai", None, ProviderModelSet::FeatherlessAi)]
#[case::deepgram("deepgram", None, ProviderModelSet::Deepgram)]
#[case::elevenlabs("elevenlabs", None, ProviderModelSet::Elevenlabs)]
#[case::heroku("heroku", None, ProviderModelSet::Heroku)]
#[case::dashscope("dashscope", None, ProviderModelSet::Dashscope)]
#[case::qwencloud("qwencloud", None, ProviderModelSet::Qwencloud)]
#[case::qwen_ai_platform("qwen_ai_platform", None, ProviderModelSet::QwenAiPlatform)]
#[case::modelscope("modelscope", None, ProviderModelSet::Modelscope)]
#[case::moonshot("moonshot", None, ProviderModelSet::Moonshot)]
#[case::publicai("publicai", None, ProviderModelSet::Publicai)]
#[case::darkbloom("darkbloom", None, ProviderModelSet::Darkbloom)]
#[case::v0("v0", None, ProviderModelSet::V0)]
#[case::morph("morph", None, ProviderModelSet::Morph)]
#[case::lambda_ai("lambda_ai", None, ProviderModelSet::LambdaAi)]
#[case::inception("inception", None, ProviderModelSet::Inception)]
#[case::hyperbolic("hyperbolic", None, ProviderModelSet::Hyperbolic)]
#[case::black_forest_labs("black_forest_labs", None, ProviderModelSet::BlackForestLabs)]
#[case::recraft("recraft", None, ProviderModelSet::Recraft)]
#[case::cometapi("cometapi", None, ProviderModelSet::Cometapi)]
#[case::oci("oci", None, ProviderModelSet::Oci)]
#[case::volcengine("volcengine", None, ProviderModelSet::Volcengine)]
#[case::wandb("wandb", None, ProviderModelSet::Wandb)]
#[case::ovhcloud("ovhcloud", None, ProviderModelSet::Ovhcloud)]
#[case::ovhcloud_embedding_models(
    "ovhcloud-embedding-models",
    None,
    ProviderModelSet::OvhcloudEmbedding
)]
#[case::lemonade("lemonade", None, ProviderModelSet::Lemonade)]
#[case::docker_model_runner("docker_model_runner", None, ProviderModelSet::DockerModelRunner)]
#[case::amazon_nova("amazon_nova", None, ProviderModelSet::AmazonNova)]
#[case::stability("stability", None, ProviderModelSet::Stability)]
#[case::github_copilot("github_copilot", None, ProviderModelSet::GithubCopilot)]
#[case::chatgpt("chatgpt", None, ProviderModelSet::Chatgpt)]
#[case::minimax("minimax", None, ProviderModelSet::Minimax)]
#[case::aws_polly("aws_polly", None, ProviderModelSet::AwsPolly)]
#[case::transcribe("transcribe", None, ProviderModelSet::Transcribe)]
#[case::gigachat("gigachat", None, ProviderModelSet::Gigachat)]
#[case::llamagate("llamagate", None, ProviderModelSet::Llamagate)]
#[case::reducto("reducto", None, ProviderModelSet::Reducto)]
#[case::bedrock_mantle("bedrock_mantle", None, ProviderModelSet::BedrockMantle)]
#[case::ai21_chat("ai21", Some(Mode::Chat), ProviderModelSet::Ai21Chat)]
#[case::ai21_other("ai21", Some(Mode::Completion), ProviderModelSet::Ai21)]
#[case::ai21_silent("ai21", None, ProviderModelSet::Ai21)]
#[case::bedrock_plain("bedrock", Some(Mode::Chat), ProviderModelSet::Bedrock)]
#[case::bedrock_silent("bedrock", None, ProviderModelSet::Bedrock)]
#[case::fireworks_ai("fireworks_ai", None, ProviderModelSet::FireworksAi)]
#[case::fireworks_ai_embedding_models(
    "fireworks_ai-embedding-models",
    None,
    ProviderModelSet::FireworksAiEmbedding
)]
fn fill_places_the_key_in_the_set_the_provider_targets(
    #[case] provider: &str,
    #[case] mode: Option<Mode>,
    #[case] expected: ProviderModelSet,
) {
    let sets = fold(&json_with_mode("the-key", provider, mode));
    assert_eq!(names(&sets, expected), ["the-key".to_owned()]);
}

#[rstest]
#[case::ft_single_colon(
    "ft:gpt-3.5-turbo",
    "openai",
    None,
    ProviderModelSet::OpenAiChatCompletion,
    false
)]
#[case::ft_two_colons(
    "ft:gpt-3.5-turbo:custom",
    "openai",
    None,
    ProviderModelSet::OpenAiChatCompletion,
    true
)]
#[case::ft_many_colons(
    "ft:gpt-3.5-turbo:org:abc",
    "openai",
    None,
    ProviderModelSet::OpenAiChatCompletion,
    true
)]
#[case::ft_prefix_only("ft:", "openai", None, ProviderModelSet::OpenAiChatCompletion, false)]
#[case::not_a_finetune_key(
    "gpt-3.5-turbo:custom",
    "openai",
    None,
    ProviderModelSet::OpenAiChatCompletion,
    true
)]
#[case::plain_bedrock("plain-model", "bedrock", None, ProviderModelSet::Bedrock, true)]
#[case::guardrail_mode(
    "guard-model",
    "bedrock",
    Some(Mode::Guardrail),
    ProviderModelSet::Bedrock,
    false
)]
#[case::chat_mode(
    "chat-model",
    "bedrock",
    Some(Mode::Chat),
    ProviderModelSet::Bedrock,
    true
)]
#[case::regional_key(
    "bedrock/us-east-1/anthropic.claude",
    "bedrock",
    None,
    ProviderModelSet::Bedrock,
    false
)]
#[case::regional_key_two_segments(
    "bedrock/eu-west-1/anthropic.claude/v1",
    "bedrock",
    None,
    ProviderModelSet::Bedrock,
    false
)]
#[case::underscore_region(
    "bedrock/us_east_1/model",
    "bedrock",
    None,
    ProviderModelSet::Bedrock,
    false
)]
#[case::hyphen_region(
    "bedrock/us-east-1-x/model",
    "bedrock",
    None,
    ProviderModelSet::Bedrock,
    false
)]
#[case::dotted_region(
    "bedrock/us.east/model",
    "bedrock",
    None,
    ProviderModelSet::Bedrock,
    true
)]
#[case::single_slash(
    "bedrock/anthropic.claude",
    "bedrock",
    None,
    ProviderModelSet::Bedrock,
    true
)]
#[case::month_commitment(
    "commitment/month-commitment-deal",
    "bedrock",
    None,
    ProviderModelSet::Bedrock,
    false
)]
#[case::empty_model("bedrock/us-east-1/", "bedrock", None, ProviderModelSet::Bedrock, true)]
#[case::fireworks_plain(
    "accounts/fireworks/models/llama",
    "fireworks_ai",
    None,
    ProviderModelSet::FireworksAi,
    true
)]
#[case::fireworks_up_to(
    "accounts/fireworks/models/llama-up-to-128k",
    "fireworks_ai",
    None,
    ProviderModelSet::FireworksAi,
    false
)]
#[case::fireworks_default(
    "fireworks-ai-default",
    "fireworks_ai",
    None,
    ProviderModelSet::FireworksAi,
    false
)]
#[case::fireworks_embedding_plain(
    "accounts/fireworks/models/embed",
    "fireworks_ai-embedding-models",
    None,
    ProviderModelSet::FireworksAiEmbedding,
    true
)]
#[case::fireworks_embedding_up_to(
    "accounts/fireworks/embedding-up-to-4k",
    "fireworks_ai-embedding-models",
    None,
    ProviderModelSet::FireworksAiEmbedding,
    false
)]
fn fill_keeps_or_skips_keys_the_chain_filters(
    #[case] key: &str,
    #[case] provider: &str,
    #[case] mode: Option<Mode>,
    #[case] set: ProviderModelSet,
    #[case] kept: bool,
) {
    let sets = fold(&json_with_mode(key, provider, mode));
    let expected: Vec<String> = if kept {
        vec![key.to_owned()]
    } else {
        Vec::new()
    };
    assert_eq!(names(&sets, set), expected);
}

#[test]
fn fill_keeps_bedrock_converse_keys_out_of_the_bedrock_set() {
    let sets = fold(r#"{"converse-model":{"litellm_provider":"bedrock_converse"}}"#);
    assert_eq!(
        names(&sets, ProviderModelSet::BedrockConverse),
        ["converse-model".to_owned()]
    );
    assert!(names(&sets, ProviderModelSet::Bedrock).is_empty());
}

#[rstest]
#[case::vertex_ai_anthropic_models("vertex_ai-anthropic_models", ProviderModelSet::VertexAnthropic)]
#[case::vertex_ai_llama_models("vertex_ai-llama_models", ProviderModelSet::VertexLlama3)]
#[case::vertex_ai_deepseek_models("vertex_ai-deepseek_models", ProviderModelSet::VertexDeepseek)]
#[case::vertex_ai_mistral_models("vertex_ai-mistral_models", ProviderModelSet::VertexMistral)]
#[case::vertex_ai_ai21_models("vertex_ai-ai21_models", ProviderModelSet::VertexAiAi21)]
#[case::vertex_ai_image_models("vertex_ai-image-models", ProviderModelSet::VertexAiImage)]
#[case::vertex_ai_video_models("vertex_ai-video-models", ProviderModelSet::VertexAiVideo)]
#[case::vertex_ai_openai_models("vertex_ai-openai_models", ProviderModelSet::VertexOpenai)]
#[case::vertex_ai_minimax_models("vertex_ai-minimax_models", ProviderModelSet::VertexMinimax)]
#[case::vertex_ai_moonshot_models("vertex_ai-moonshot_models", ProviderModelSet::VertexMoonshot)]
#[case::vertex_ai_zai_models("vertex_ai-zai_models", ProviderModelSet::VertexZai)]
fn fill_strips_the_vertex_prefix(#[case] provider: &str, #[case] expected: ProviderModelSet) {
    let sets = fold(&catalog_of(&[("vertex_ai/claude-model", provider)]));
    assert_eq!(names(&sets, expected), ["claude-model".to_owned()]);
}

#[test]
fn fill_strips_every_vertex_prefix_occurrence_like_str_replace() {
    let sets = fold(&catalog_of(&[(
        "vertex_ai/vertex_ai/nested",
        "vertex_ai-anthropic_models",
    )]));
    assert_eq!(
        names(&sets, ProviderModelSet::VertexAnthropic),
        ["nested".to_owned()]
    );
}

#[rstest]
#[case::unknown_provider("not-a-provider")]
#[case::empty_provider("")]
fn fill_ignores_providers_the_chain_does_not_name(#[case] provider: &str) {
    let sets = fold(&catalog_of(&[("orphan", provider)]));
    assert!(sets.sets.values().all(BTreeSet::is_empty));
    assert!(sets.models_by_provider.values().all(BTreeSet::is_empty));
}

#[test]
fn fill_accumulates_across_calls() {
    let mut sets = fold(r#"{"first":{"litellm_provider":"openai"}}"#);
    sets.add_known_models(
        &ModelCatalog::parse(
            br#"{"second":{"litellm_provider":"openai"}}"#,
            UnknownFields::Warn,
        )
        .unwrap(),
    );
    assert_eq!(
        names(&sets, ProviderModelSet::OpenAiChatCompletion),
        ["first".to_owned(), "second".to_owned()]
    );
}

#[test]
fn by_provider_keys_are_all_present_and_empty_before_a_fold() {
    let sets = ProviderModelSets::default();
    let keys: Vec<&str> = sets.models_by_provider.keys().map(String::as_str).collect();
    assert_eq!(keys, BY_PROVIDER_KEYS);
    assert!(sets.models_by_provider.values().all(BTreeSet::is_empty));
}

#[test]
fn by_provider_unions_the_openai_chat_and_text_sets() {
    let sets = fold(&catalog_of(&[
        ("chat-model", "openai"),
        ("text-model", "text-completion-openai"),
    ]));
    assert_eq!(
        provider_names(&sets, "openai"),
        ["chat-model".to_owned(), "text-model".to_owned()]
    );
    assert_eq!(
        provider_names(&sets, "text-completion-openai"),
        ["text-model".to_owned()]
    );
}

#[test]
fn by_provider_unions_the_cohere_sets() {
    let sets = fold(&catalog_of(&[
        ("cohere-model", "cohere"),
        ("cohere-chat-model", "cohere_chat"),
    ]));
    assert_eq!(
        provider_names(&sets, "cohere"),
        ["cohere-chat-model".to_owned(), "cohere-model".to_owned()]
    );
    assert_eq!(
        provider_names(&sets, "cohere_chat"),
        ["cohere-chat-model".to_owned()]
    );
}

#[test]
fn by_provider_vertex_union_takes_only_the_nine_member_sets() {
    let sets = fold(&catalog_of(&[
        ("vertex-chat", "vertex_ai-chat-models"),
        ("vertex-text", "vertex_ai-text-models"),
        ("vertex-anthropic", "vertex_ai-anthropic_models"),
        ("vertex-vision", "vertex_ai-vision-models"),
        ("vertex-language", "vertex_ai-language-models"),
        ("vertex-deepseek", "vertex_ai-deepseek_models"),
        ("vertex-minimax", "vertex_ai-minimax_models"),
        ("vertex-moonshot", "vertex_ai-moonshot_models"),
        ("vertex-zai", "vertex_ai-zai_models"),
        ("vertex-image", "vertex_ai-image-models"),
        ("vertex-video", "vertex_ai-video-models"),
        ("vertex-openai", "vertex_ai-openai_models"),
        ("vertex-llama", "vertex_ai-llama_models"),
        ("vertex-mistral", "vertex_ai-mistral_models"),
        ("vertex-ai21", "vertex_ai-ai21_models"),
        ("vertex-code-chat", "vertex_ai-code-chat-models"),
        ("vertex-code-text", "vertex_ai-code-text-models"),
        ("vertex-embedding", "vertex_ai-embedding-models"),
    ]));
    assert_eq!(
        provider_names(&sets, "vertex_ai"),
        [
            "vertex-anthropic".to_owned(),
            "vertex-chat".to_owned(),
            "vertex-deepseek".to_owned(),
            "vertex-language".to_owned(),
            "vertex-minimax".to_owned(),
            "vertex-moonshot".to_owned(),
            "vertex-text".to_owned(),
            "vertex-vision".to_owned(),
            "vertex-zai".to_owned(),
        ]
    );
}

#[test]
fn by_provider_unions_bedrock_with_its_converse_set() {
    let sets = fold(&catalog_of(&[
        ("bedrock-model", "bedrock"),
        ("bedrock-converse-model", "bedrock_converse"),
    ]));
    assert_eq!(
        provider_names(&sets, "bedrock"),
        [
            "bedrock-converse-model".to_owned(),
            "bedrock-model".to_owned()
        ]
    );
}

#[test]
fn by_provider_unions_the_embedding_siblings() {
    let sets = fold(&catalog_of(&[
        ("azure-model", "azure"),
        ("azure-text-model", "azure_text"),
        ("fireworks-model", "fireworks_ai"),
        ("fireworks-embedding", "fireworks_ai-embedding-models"),
        ("sambanova-model", "sambanova"),
        ("sambanova-embedding", "sambanova-embedding-models"),
        ("nebius-model", "nebius-chat-models"),
        ("nebius-embedding", "nebius-embedding-models"),
        ("ovhcloud-model", "ovhcloud"),
        ("ovhcloud-embedding", "ovhcloud-embedding-models"),
    ]));
    assert_eq!(
        provider_names(&sets, "azure"),
        ["azure-model".to_owned(), "azure-text-model".to_owned()]
    );
    assert_eq!(
        provider_names(&sets, "fireworks_ai"),
        [
            "fireworks-embedding".to_owned(),
            "fireworks-model".to_owned()
        ]
    );
    assert_eq!(
        provider_names(&sets, "sambanova"),
        [
            "sambanova-embedding".to_owned(),
            "sambanova-model".to_owned()
        ]
    );
    assert_eq!(
        provider_names(&sets, "nebius"),
        ["nebius-embedding".to_owned(), "nebius-model".to_owned()]
    );
    assert_eq!(
        provider_names(&sets, "ovhcloud"),
        ["ovhcloud-embedding".to_owned(), "ovhcloud-model".to_owned()]
    );
}

#[test]
fn by_provider_ai21_key_omits_the_chat_set_like_the_source() {
    let sets = fold(
        r#"{"ai21-plain":{"litellm_provider":"ai21"},"ai21-chat":{"litellm_provider":"ai21","mode":"chat"}}"#,
    );
    assert_eq!(provider_names(&sets, "ai21"), ["ai21-plain".to_owned()]);
    assert_eq!(
        names(&sets, ProviderModelSet::Ai21Chat),
        ["ai21-chat".to_owned()]
    );
}

#[test]
fn by_provider_rebuilds_after_a_second_fold() {
    let mut sets = fold(&catalog_of(&[("first", "deepseek")]));
    assert_eq!(provider_names(&sets, "deepseek"), ["first".to_owned()]);
    sets.add_known_models(
        &ModelCatalog::parse(
            br#"{"second":{"litellm_provider":"deepseek"}}"#,
            UnknownFields::Warn,
        )
        .unwrap(),
    );
    assert_eq!(
        provider_names(&sets, "deepseek"),
        ["first".to_owned(), "second".to_owned()]
    );
}
