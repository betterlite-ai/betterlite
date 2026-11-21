from betterlite import _native
from betterlite._settings.constant_sets import EXTRA_MODELS

_SETS: dict[str, list[str]] = _native.provider_model_sets()

open_ai_chat_completion_models: set[str] = set(_SETS["open_ai_chat_completion_models"])
open_ai_text_completion_models: set[str] = set(_SETS["open_ai_text_completion_models"])
cohere_models: set[str] = set(_SETS["cohere_models"])
cohere_chat_models: set[str] = set(_SETS["cohere_chat_models"])
mistral_chat_models: set[str] = set(_SETS["mistral_chat_models"])
# ast-grep-ignore: no-long-names
text_completion_codestral_models: set[str] = set(_SETS["text_completion_codestral_models"])
# ast-grep-ignore: no-long-names
text_completion_inception_models: set[str] = set(_SETS["text_completion_inception_models"])
anthropic_models: set[str] = set(_SETS["anthropic_models"])
openrouter_models: set[str] = set(_SETS["openrouter_models"])
datarobot_models: set[str] = set(_SETS["datarobot_models"])
vertex_language_models: set[str] = set(_SETS["vertex_language_models"])
vertex_vision_models: set[str] = set(_SETS["vertex_vision_models"])
vertex_chat_models: set[str] = set(_SETS["vertex_chat_models"])
vertex_code_chat_models: set[str] = set(_SETS["vertex_code_chat_models"])
vertex_ai_image_models: set[str] = set(_SETS["vertex_ai_image_models"])
vertex_ai_video_models: set[str] = set(_SETS["vertex_ai_video_models"])
vertex_text_models: set[str] = set(_SETS["vertex_text_models"])
vertex_code_text_models: set[str] = set(_SETS["vertex_code_text_models"])
vertex_embedding_models: set[str] = set(_SETS["vertex_embedding_models"])
vertex_anthropic_models: set[str] = set(_SETS["vertex_anthropic_models"])
vertex_llama3_models: set[str] = set(_SETS["vertex_llama3_models"])
vertex_deepseek_models: set[str] = set(_SETS["vertex_deepseek_models"])
vertex_ai_ai21_models: set[str] = set(_SETS["vertex_ai_ai21_models"])
vertex_mistral_models: set[str] = set(_SETS["vertex_mistral_models"])
vertex_openai_models: set[str] = set(_SETS["vertex_openai_models"])
vertex_minimax_models: set[str] = set(_SETS["vertex_minimax_models"])
vertex_moonshot_models: set[str] = set(_SETS["vertex_moonshot_models"])
vertex_zai_models: set[str] = set(_SETS["vertex_zai_models"])
ai21_models: set[str] = set(_SETS["ai21_models"])
ai21_chat_models: set[str] = set(_SETS["ai21_chat_models"])
nlp_cloud_models: set[str] = set(_SETS["nlp_cloud_models"])
aleph_alpha_models: set[str] = set(_SETS["aleph_alpha_models"])
bedrock_models: set[str] = set(_SETS["bedrock_models"])
bedrock_converse_models: set[str] = set(_SETS["bedrock_converse_models"]) | set(
    EXTRA_MODELS["bedrock_converse_models"]
)
fal_ai_models: set[str] = set(_SETS["fal_ai_models"])
fireworks_ai_models: set[str] = set(_SETS["fireworks_ai_models"])
fireworks_ai_embedding_models: set[str] = set(_SETS["fireworks_ai_embedding_models"])
deepinfra_models: set[str] = set(_SETS["deepinfra_models"])
perplexity_models: set[str] = set(_SETS["perplexity_models"])
watsonx_models: set[str] = set(_SETS["watsonx_models"])
gemini_models: set[str] = set(_SETS["gemini_models"])
xai_models: set[str] = set(_SETS["xai_models"])
zai_models: set[str] = set(_SETS["zai_models"])
deepseek_models: set[str] = set(_SETS["deepseek_models"])
tencent_models: set[str] = set(_SETS["tencent_models"])
runwayml_models: set[str] = set(_SETS["runwayml_models"])
azure_ai_models: set[str] = set(_SETS["azure_ai_models"])
jina_ai_models: set[str] = set(_SETS["jina_ai_models"])
voyage_models: set[str] = set(_SETS["voyage_models"])
infinity_models: set[str] = set(_SETS["infinity_models"])
heroku_models: set[str] = set(_SETS["heroku_models"])
databricks_models: set[str] = set(_SETS["databricks_models"])
cloudflare_models: set[str] = set(_SETS["cloudflare_models"])
codestral_models: set[str] = set(_SETS["codestral_models"])
friendliai_models: set[str] = set(_SETS["friendliai_models"])
featherless_ai_models: set[str] = set(_SETS["featherless_ai_models"])
palm_models: set[str] = set(_SETS["palm_models"])
groq_models: set[str] = set(_SETS["groq_models"])
azure_models: set[str] = set(_SETS["azure_models"])
azure_anthropic_models: set[str] = set(_SETS["azure_anthropic_models"])
azure_text_models: set[str] = set(_SETS["azure_text_models"])
anyscale_models: set[str] = set(_SETS["anyscale_models"])
cerebras_models: set[str] = set(_SETS["cerebras_models"])
nadir_models: set[str] = set(_SETS["nadir_models"])
galadriel_models: set[str] = set(_SETS["galadriel_models"])
nvidia_nim_models: set[str] = set(_SETS["nvidia_nim_models"])
nvidia_riva_models: set[str] = set(_SETS["nvidia_riva_models"])
soniox_models: set[str] = set(_SETS["soniox_models"])
sambanova_models: set[str] = set(_SETS["sambanova_models"])
sambanova_embedding_models: set[str] = set(_SETS["sambanova_embedding_models"])
novita_models: set[str] = set(_SETS["novita_models"])
assemblyai_models: set[str] = set(_SETS["assemblyai_models"])
snowflake_models: set[str] = set(_SETS["snowflake_models"])
gradient_ai_models: set[str] = set(_SETS["gradient_ai_models"])
llama_models: set[str] = set(_SETS["llama_models"])
nscale_models: set[str] = set(_SETS["nscale_models"])
nebius_models: set[str] = set(_SETS["nebius_models"])
nebius_embedding_models: set[str] = set(_SETS["nebius_embedding_models"])
aiml_models: set[str] = set(_SETS["aiml_models"])
deepgram_models: set[str] = set(_SETS["deepgram_models"])
elevenlabs_models: set[str] = set(_SETS["elevenlabs_models"])
dashscope_models: set[str] = set(_SETS["dashscope_models"])
qwencloud_models: set[str] = set(_SETS["qwencloud_models"])
qwen_ai_platform_models: set[str] = set(_SETS["qwen_ai_platform_models"])
moonshot_models: set[str] = set(_SETS["moonshot_models"])
publicai_models: set[str] = set(_SETS["publicai_models"])
darkbloom_models: set[str] = set(_SETS["darkbloom_models"])
v0_models: set[str] = set(_SETS["v0_models"])
morph_models: set[str] = set(_SETS["morph_models"])
lambda_ai_models: set[str] = set(_SETS["lambda_ai_models"])
inception_models: set[str] = set(_SETS["inception_models"])
hyperbolic_models: set[str] = set(_SETS["hyperbolic_models"])
black_forest_labs_models: set[str] = set(_SETS["black_forest_labs_models"])
recraft_models: set[str] = set(_SETS["recraft_models"])
cometapi_models: set[str] = set(_SETS["cometapi_models"])
oci_models: set[str] = set(_SETS["oci_models"])
vercel_ai_gateway_models: set[str] = set(_SETS["vercel_ai_gateway_models"])
edenai_models: set[str] = set(_SETS["edenai_models"])
volcengine_models: set[str] = set(_SETS["volcengine_models"])
wandb_models: set[str] = set(_SETS["wandb_models"]) | set(
    EXTRA_MODELS["wandb_models"]
)
ovhcloud_models: set[str] = set(_SETS["ovhcloud_models"])
ovhcloud_embedding_models: set[str] = set(_SETS["ovhcloud_embedding_models"])
lemonade_models: set[str] = set(_SETS["lemonade_models"])
docker_model_runner_models: set[str] = set(_SETS["docker_model_runner_models"])
amazon_nova_models: set[str] = set(_SETS["amazon_nova_models"])
stability_models: set[str] = set(_SETS["stability_models"])
github_copilot_models: set[str] = set(_SETS["github_copilot_models"])
chatgpt_models: set[str] = set(_SETS["chatgpt_models"])
minimax_models: set[str] = set(_SETS["minimax_models"])
aws_polly_models: set[str] = set(_SETS["aws_polly_models"])
transcribe_models: set[str] = set(_SETS["transcribe_models"])
gigachat_models: set[str] = set(_SETS["gigachat_models"])
llamagate_models: set[str] = set(_SETS["llamagate_models"])
reducto_models: set[str] = set(_SETS["reducto_models"])
bedrock_mantle_models: set[str] = set(_SETS["bedrock_mantle_models"])
petals_models: list[str] = list(EXTRA_MODELS["petals_models"])
ollama_models: list[str] = list(EXTRA_MODELS["ollama_models"])
maritalk_models: list[str] = list(EXTRA_MODELS["maritalk_models"])


__all__ = [
    "ai21_chat_models", "ai21_models", "aiml_models", "aleph_alpha_models", "amazon_nova_models",
    "anthropic_models", "anyscale_models", "assemblyai_models", "aws_polly_models",
    "azure_ai_models", "azure_anthropic_models", "azure_models", "azure_text_models",
    "bedrock_converse_models", "bedrock_mantle_models", "bedrock_models",
    "black_forest_labs_models", "cerebras_models", "chatgpt_models", "cloudflare_models",
    "codestral_models", "cohere_chat_models", "cohere_models", "cometapi_models",
    "darkbloom_models", "dashscope_models", "databricks_models", "datarobot_models",
    "deepgram_models", "deepinfra_models", "deepseek_models", "docker_model_runner_models",
    "edenai_models", "elevenlabs_models", "fal_ai_models", "featherless_ai_models",
    "fireworks_ai_embedding_models", "fireworks_ai_models", "friendliai_models", "galadriel_models",
    "gemini_models", "gigachat_models", "github_copilot_models", "gradient_ai_models",
    "groq_models", "heroku_models", "hyperbolic_models", "inception_models", "infinity_models",
    "jina_ai_models", "lambda_ai_models", "lemonade_models", "llama_models", "llamagate_models",
    "maritalk_models", "minimax_models", "mistral_chat_models", "moonshot_models", "morph_models",
    "nadir_models", "nebius_embedding_models", "nebius_models", "nlp_cloud_models", "novita_models",
    "nscale_models", "nvidia_nim_models", "nvidia_riva_models", "oci_models", "ollama_models",
    "open_ai_chat_completion_models", "open_ai_text_completion_models", "openrouter_models",
    "ovhcloud_embedding_models", "ovhcloud_models", "palm_models", "perplexity_models",
    "petals_models", "publicai_models", "qwen_ai_platform_models", "qwencloud_models",
    "recraft_models", "reducto_models", "runwayml_models", "sambanova_embedding_models",
    "sambanova_models", "snowflake_models", "soniox_models", "stability_models", "tencent_models",
    "text_completion_codestral_models", "text_completion_inception_models", "transcribe_models",
    "v0_models", "vercel_ai_gateway_models", "vertex_ai_ai21_models", "vertex_ai_image_models",
    "vertex_ai_video_models", "vertex_anthropic_models", "vertex_chat_models",
    "vertex_code_chat_models", "vertex_code_text_models", "vertex_deepseek_models",
    "vertex_embedding_models", "vertex_language_models", "vertex_llama3_models",
    "vertex_minimax_models", "vertex_mistral_models", "vertex_moonshot_models",
    "vertex_openai_models", "vertex_text_models", "vertex_vision_models", "vertex_zai_models",
    "volcengine_models", "voyage_models", "wandb_models", "watsonx_models", "xai_models",
    "zai_models",
]
