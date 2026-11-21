from betterlite._settings import constant_sets, sets

azure_llms = {
    "gpt-35-turbo": "azure/gpt-35-turbo", "gpt-35-turbo-16k": "azure/gpt-35-turbo-16k",
    "gpt-35-turbo-instruct": "azure/gpt-35-turbo-instruct", "azure/gpt-41": "gpt-4.1",
    "azure/gpt-41-mini": "gpt-4.1-mini", "azure/gpt-41-nano": "gpt-4.1-nano",
}
azure_embedding_models = {"ada": "azure/ada"}
model_list: list[str] = list(
    sets.open_ai_chat_completion_models | sets.open_ai_text_completion_models | sets.cohere_models
    | sets.cohere_chat_models | sets.anthropic_models | set(constant_sets.replicate_models)
    | sets.openrouter_models | sets.datarobot_models | set(constant_sets.huggingface_models)
    | sets.vertex_chat_models | sets.vertex_text_models | sets.ai21_models | sets.ai21_chat_models
    | set(constant_sets.together_ai_models) | set(constant_sets.baseten_models)
    | sets.aleph_alpha_models | sets.nlp_cloud_models | set(sets.ollama_models)
    | sets.bedrock_models | sets.deepinfra_models | sets.perplexity_models
    | set(sets.maritalk_models) | sets.runwayml_models | sets.vertex_language_models
    | sets.watsonx_models | sets.gemini_models | sets.text_completion_codestral_models
    | sets.text_completion_inception_models | sets.xai_models | sets.zai_models | sets.fal_ai_models
    | sets.deepseek_models | constant_sets.modelscope_models | sets.azure_ai_models
    | sets.voyage_models | sets.infinity_models | sets.databricks_models | sets.cloudflare_models
    | sets.codestral_models | sets.friendliai_models | sets.palm_models | sets.groq_models
    | sets.azure_models | sets.azure_anthropic_models | sets.anyscale_models | sets.cerebras_models
    | sets.nadir_models | sets.galadriel_models | sets.nvidia_nim_models | sets.nvidia_riva_models
    | sets.soniox_models | sets.sambanova_models | sets.azure_text_models | sets.novita_models
    | sets.assemblyai_models | sets.jina_ai_models | sets.snowflake_models | sets.gradient_ai_models
    | sets.llama_models | sets.featherless_ai_models | sets.nscale_models | sets.deepgram_models
    | sets.elevenlabs_models | sets.dashscope_models | sets.qwencloud_models
    | sets.qwen_ai_platform_models | sets.moonshot_models | sets.publicai_models
    | sets.darkbloom_models | sets.v0_models | sets.morph_models | sets.lambda_ai_models
    | sets.inception_models | sets.black_forest_labs_models | sets.recraft_models
    | sets.cometapi_models | sets.oci_models | sets.heroku_models | sets.vercel_ai_gateway_models
    | sets.edenai_models | sets.volcengine_models | sets.wandb_models | sets.ovhcloud_models
    | sets.lemonade_models | sets.docker_model_runner_models | sets.reducto_models
    | sets.bedrock_mantle_models | set(constant_sets.clarifai_models)
)
model_list_set: set[str] = set(model_list)


__all__ = [
    "azure_embedding_models", "azure_llms", "model_list", "model_list_set",
]
