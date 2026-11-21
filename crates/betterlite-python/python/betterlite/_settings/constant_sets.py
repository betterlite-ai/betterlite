from betterlite import _native

EXTRA_MODELS: dict[str, list[str]] = {
    "baseten_models": ["31dxrj3", "q841o8w", "qvv0xeq"],
    "bedrock_converse_models": [
        "ai21.jamba-1-5-large-v1:0", "ai21.jamba-1-5-mini-v1:0", "ai21.jamba-instruct-v1:0",
        "anthropic.claude-3-5-haiku-20241022-v1:0", "anthropic.claude-3-5-sonnet-20240620-v1:0",
        "anthropic.claude-3-5-sonnet-20241022-v2:0", "anthropic.claude-3-haiku-20240307-v1:0",
        "anthropic.claude-3-opus-20240229-v1:0", "anthropic.claude-3-sonnet-20240229-v1:0",
        "anthropic.claude-instant-v1", "anthropic.claude-opus-4-6-v1:0", "anthropic.claude-v1",
        "anthropic.claude-v2", "anthropic.claude-v2:1", "meta.llama3-1-405b-instruct-v1:0",
        "meta.llama3-1-70b-instruct-v1:0", "meta.llama3-1-8b-instruct-v1:0",
        "meta.llama3-2-11b-instruct-v1:0", "meta.llama3-2-1b-instruct-v1:0",
        "meta.llama3-2-3b-instruct-v1:0", "meta.llama3-2-90b-instruct-v1:0",
        "meta.llama3-70b-instruct-v1:0", "meta.llama3-8b-instruct-v1:0",
        "mistral.mistral-large-2402-v1:0", "mistral.mistral-large-2407-v1:0",
        "mistral.mistral-small-2402-v1:0",
    ],
    "clarifai_models": [
        "clarifai/anthropic.completion.claude-3_5-haiku",
        "clarifai/anthropic.completion.claude-3_7-sonnet",
        "clarifai/anthropic.completion.claude-opus-4",
        "clarifai/anthropic.completion.claude-sonnet-4",
        "clarifai/deepseek-ai.deepseek-chat.DeepSeek-R1-0528-Qwen3-8B",
        "clarifai/gcp.generate.gemini-2_5-pro", "clarifai/meta.Llama-3.Llama-3_2-3B-Instruct",
        "clarifai/microsoft.text-generation.Phi-4-reasoning-plus",
        "clarifai/microsoft.text-generation.phi-4", "clarifai/openai.chat-completion.gpt-4o",
        "clarifai/openai.chat-completion.gpt-5", "clarifai/openai.chat-completion.gpt-5-mini",
        "clarifai/openai.chat-completion.gpt-oss-120b",
        "clarifai/openai.chat-completion.gpt-oss-20b", "clarifai/openai.chat-completion.o3",
        "clarifai/openbmb.miniCPM.MiniCPM-o-2_6-language", "clarifai/openbmb.miniCPM.MiniCPM3-4B",
        "clarifai/openbmb.miniCPM.MiniCPM4-8B", "clarifai/qwen.qwen-VL.Qwen2_5-VL-7B-Instruct",
        "clarifai/qwen.qwen3.qwen3-next-80B-A3B-Thinking",
        "clarifai/qwen.qwenCoder.Qwen3-Coder-30B-A3B-Instruct", "clarifai/qwen.qwenLM.QwQ-32B-AWQ",
        "clarifai/qwen.qwenLM.Qwen3-14B", "clarifai/qwen.qwenLM.Qwen3-30B-A3B-Instruct-2507",
        (
            "clarifai/qwen.qwenLM.Qwen3-30B-A3B-Thinking-2507clarifai/openai.chat-completion.gpt-"
            "5-nano"
        ),
        "clarifai/xai.chat-completion.grok-2-1212",
        "clarifai/xai.chat-completion.grok-2-vision-1212", "clarifai/xai.chat-completion.grok-3",
        "clarifai/xai.chat-completion.grok-code-fast-1",
        "clarifai/xai.image-generation.grok-2-image-1212",
    ],
    "empower_models": ["empower/empower-functions", "empower/empower-functions-small"],
    "huggingface_models": [
        "meta-llama/Llama-2-13b", "meta-llama/Llama-2-13b-chat", "meta-llama/Llama-2-13b-chat-hf",
        "meta-llama/Llama-2-13b-hf", "meta-llama/Llama-2-70b", "meta-llama/Llama-2-70b-chat",
        "meta-llama/Llama-2-70b-chat-hf", "meta-llama/Llama-2-70b-hf", "meta-llama/Llama-2-7b",
        "meta-llama/Llama-2-7b-chat", "meta-llama/Llama-2-7b-chat-hf", "meta-llama/Llama-2-7b-hf",
    ],
    "maritalk_models": ["maritalk"],
    "modelscope_models": [
        "Qwen/QVQ-72B-Preview", "Qwen/QwQ-32B", "Qwen/QwQ-32B-Preview", "Qwen/Qwen-Image-Edit",
        "Qwen/Qwen3-0.6B", "Qwen/Qwen3-1.7B", "Qwen/Qwen3-14B", "Qwen/Qwen3-235B-A22B",
        "Qwen/Qwen3-235B-A22B-Instruct-2507", "Qwen/Qwen3-235B-A22B-Thinking-2507",
        "Qwen/Qwen3-30B-A3B", "Qwen/Qwen3-30B-A3B-Thinking-2507", "Qwen/Qwen3-32B", "Qwen/Qwen3-4B",
        "Qwen/Qwen3-8B", "Qwen/Qwen3-Coder-30B-A3B-Instruct", "Qwen/Qwen3-Coder-480B-A35B-Instruct",
        "Qwen/Qwen3-Next-80B-A3B-Instruct", "Qwen/Qwen3-Next-80B-A3B-Thinking",
        "Qwen/Qwen3-VL-235B-A22B-Instruct", "Qwen/Qwen3-VL-8B-Instruct",
        "Qwen/Qwen3-VL-8B-Thinking", "Qwen/Qwen3.5-122B-A10B", "Qwen/Qwen3.5-27B",
        "Qwen/Qwen3.5-35B-A3B", "Qwen/Qwen3.5-397B-A17B", "deepseek-ai/DeepSeek-R1-0528",
        "deepseek-ai/DeepSeek-R1-Distill-Llama-70B", "deepseek-ai/DeepSeek-R1-Distill-Llama-8B",
        "deepseek-ai/DeepSeek-R1-Distill-Qwen-1.5B", "deepseek-ai/DeepSeek-R1-Distill-Qwen-14B",
        "deepseek-ai/DeepSeek-R1-Distill-Qwen-32B", "deepseek-ai/DeepSeek-R1-Distill-Qwen-7B",
        "deepseek-ai/DeepSeek-V3.2", "deepseek-ai/DeepSeek-V4-Flash",
    ],
    "ollama_models": ["llama2"], "petals_models": ["petals-team/StableBeluga2"],
    "replicate_models": [
        (
            "a16z-infra/llama-2-13b-chat:2a7f981751ec7fdf87b5b91ad4db53683a98082e9ff7bfd12c8cd5ea"
            "85980a52"
        ),
        "daanelson/flan-t5-large:ce962b3f6792a57074a601d3979db5839697add2e4e02696b3ced4c022d4767f",
        (
            "joehoover/instructblip-vicuna13b:c4c54e3c8c97cd50c2d2fec9be3b6065563ccf7d43787fb99f8"
            "4151b867178fe"
        ),
        "meta/codellama-13b:1c914d844307b0588599b8393480a3ba917b660c7e9dfae681542b5325f228db",
        "replicate/dolly-v2-12b:ef0e1aefc61f8e096ebe4db6b2bacc297daf2ef6899f0f7e001ec445893500e5",
        (
            "replicate/llama-2-70b-chat:2796ee9483c3fd7aa2e171d38f4ca12251a30609463dcfd4cd76703f2"
            "2e96cdf"
        ),
        "replicate/vicuna-13b:6282abe6a492de4145d7bb601023762212f9ddbbe78278bd6771c8b3b2f2a13b",
        "replit/replit-code-v1-3b:b84f4c074b807211cd75e3e8b1589b6399052125b4c27106e43d47189e8415ad",
    ],
    "together_ai_models": [
        "Austism/chronos-hermes-13b", "HuggingFaceH4/starchat-alpha",
        "NousResearch/Nous-Hermes-Llama2-13b", "NumbersStation/nsql-llama-2-7B",
        "WizardLM/WizardCoder-15B-V1.0", "WizardLM/WizardCoder-Python-34B-V1.0",
        "WizardLM/WizardLM-70B-V1.0", "defog/sqlcoder", "togethercomputer/CodeLlama-34b",
        "togethercomputer/CodeLlama-34b-Instruct", "togethercomputer/CodeLlama-34b-Python",
        "togethercomputer/LLaMA-2-7B-32K", "togethercomputer/Llama-2-7B-32K-Instruct",
        "togethercomputer/alpaca-7b", "togethercomputer/falcon-40b-instruct",
        "togethercomputer/falcon-7b-instruct", "togethercomputer/llama-2-70b",
        "togethercomputer/llama-2-70b-chat", "togethercomputer/llama-2-7b",
        "upstage/SOLAR-0-70b-16bit",
    ],
    "wandb_models": [
        "MiniMaxAI/MiniMax-M2.5", "Qwen/Qwen3-235B-A22B-Instruct-2507",
        "Qwen/Qwen3-235B-A22B-Thinking-2507", "Qwen/Qwen3-Coder-480B-A35B-Instruct",
        "deepseek-ai/DeepSeek-R1-0528", "deepseek-ai/DeepSeek-V3-0324", "deepseek-ai/DeepSeek-V3.1",
        "meta-llama/Llama-3.1-8B-Instruct", "meta-llama/Llama-3.3-70B-Instruct",
        "meta-llama/Llama-4-Scout-17B-16E-Instruct", "microsoft/Phi-4-mini-instruct",
        "moonshotai/Kimi-K2-Instruct", "moonshotai/Kimi-K2.5", "openai/gpt-oss-120b",
        "openai/gpt-oss-20b", "zai-org/GLM-4.5",
    ],
}

_SETS: dict[str, list[str]] = _native.provider_model_sets()

replicate_models: set[str] = set(_SETS["replicate_models"]) | set(
    EXTRA_MODELS["replicate_models"]
)
clarifai_models: set[str] = set(_SETS["clarifai_models"]) | set(
    EXTRA_MODELS["clarifai_models"]
)
huggingface_models: set[str] = set(_SETS["huggingface_models"]) | set(
    EXTRA_MODELS["huggingface_models"]
)
modelscope_models: set[str] = set(_SETS["modelscope_models"]) | set(
    EXTRA_MODELS["modelscope_models"]
)
empower_models: set[str] = set(_SETS["empower_models"]) | set(
    EXTRA_MODELS["empower_models"]
)
together_ai_models: set[str] = set(_SETS["together_ai_models"]) | set(
    EXTRA_MODELS["together_ai_models"]
)
baseten_models: set[str] = set(_SETS["baseten_models"]) | set(
    EXTRA_MODELS["baseten_models"]
)
open_ai_embedding_models: set[str] = {"text-embedding-ada-002"}
cohere_embedding_models: set[str] = {
    "embed-english-light-v2.0", "embed-english-light-v3.0", "embed-english-v2.0",
    "embed-english-v3.0", "embed-multilingual-v2.0", "embed-multilingual-v3.0", "embed-v4.0",
}
bedrock_embedding_models: set[str] = {
    "amazon.nova-2-multimodal-embeddings-v1:0", "amazon.titan-embed-text-v1",
    "cohere.embed-english-v3", "cohere.embed-multilingual-v3", "cohere.embed-v4:0",
    "twelvelabs.marengo-embed-2-7-v1:0", "twelvelabs.marengo-embed-3-0-v1:0",
}


__all__ = [
    "baseten_models", "bedrock_embedding_models", "clarifai_models", "cohere_embedding_models",
    "empower_models", "huggingface_models", "modelscope_models", "open_ai_embedding_models",
    "replicate_models", "together_ai_models",
]
