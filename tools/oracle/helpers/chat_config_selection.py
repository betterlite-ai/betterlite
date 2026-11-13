from collections.abc import Callable
from contextlib import ExitStack
from importlib import import_module
from typing import Final, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.azure.chat.gpt_5_transformation import AzureOpenAIGPT5Config
from litellm.llms.azure.chat.gpt_transformation import AzureOpenAIConfig
from litellm.llms.azure.chat.o_series_transformation import AzureOpenAIO1Config
from litellm.llms.azure_ai.common_utils import AzureFoundryModelInfo
from litellm.llms.bedrock.chat.invoke_transformations.anthropic_claude2_transformation import (
    AmazonAnthropicConfig,
)
from litellm.llms.openai.chat.gpt_5_transformation import OpenAIGPT5Config
from litellm.llms.openai_like import dynamic_config
from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_CLASSES: Final[tuple[str, ...]] = (
    "A2AConfig",
    "AI21ChatConfig",
    "AiohttpOpenAIChatConfig",
    "AmazonNovaChatConfig",
    "AnthropicConfig",
    "AnthropicTextConfig",
    "AzureOpenAITextConfig",
    "BasetenConfig",
    "BedrockMantleChatConfig",
    "BytezChatConfig",
    "CerebrasConfig",
    "ChatGPTConfig",
    "ClarifaiConfig",
    "CloudflareChatConfig",
    "CodestralTextCompletionConfig",
    "CometAPIConfig",
    "CompactifAIChatConfig",
    "DashScopeChatConfig",
    "DataRobotConfig",
    "DatabricksConfig",
    "DeepInfraConfig",
    "DeepSeekChatConfig",
    "DockerModelRunnerChatConfig",
    "EdenAIChatConfig",
    "EmpowerChatConfig",
    "FalAIChatConfig",
    "FeatherlessAIConfig",
    "FireworksAIConfig",
    "FriendliaiChatConfig",
    "GDCGeminiConfig",
    "GaladrielChatConfig",
    "GenAIHubOrchestrationConfig",
    "GigaChatConfig",
    "GithubChatConfig",
    "GithubCopilotConfig",
    "GoogleAIStudioGeminiConfig",
    "GradientAIConfig",
    "GroqChatConfig",
    "HerokuChatConfig",
    "HostedVLLMChatConfig",
    "HuggingFaceChatConfig",
    "HyperbolicChatConfig",
    "IBMWatsonXAIConfig",
    "IBMWatsonXChatConfig",
    "InceptionChatConfig",
    "InceptionTextCompletionConfig",
    "LMStudioChatConfig",
    "LambdaAIChatConfig",
    "LiteLLMProxyChatConfig",
    "LlamaAPIConfig",
    "LlamafileChatConfig",
    "MaritalkConfig",
    "MinimaxChatConfig",
    "MistralConfig",
    "ModelScopeChatConfig",
    "MoonshotChatConfig",
    "MorphChatConfig",
    "NLPCloudConfig",
    "NadirConfig",
    "NebiusConfig",
    "NovitaConfig",
    "NscaleConfig",
    "NvidiaNimConfig",
    "OCIChatConfig",
    "OVHCloudChatConfig",
    "OllamaChatConfig",
    "OllamaConfig",
    "OobaboogaConfig",
    "OpenAIGPTConfig",
    "OpenAILikeChatConfig",
    "OpenAITextCompletionConfig",
    "OpenrouterConfig",
    "PerplexityChatConfig",
    "PetalsConfig",
    "PredibaseConfig",
    "QwenAIPlatformChatConfig",
    "QwenCloudChatConfig",
    "RAGFlowConfig",
    "ReplicateConfig",
    "SagemakerChatConfig",
    "SagemakerConfig",
    "SagemakerNovaConfig",
    "SambanovaConfig",
    "SnowflakeConfig",
    "TencentChatConfig",
    "TogetherAIChatConfig",
    "TritonConfig",
    "V0ChatConfig",
    "VLLMConfig",
    "VercelAIGatewayConfig",
    "VertexGeminiConfig",
    "VolcEngineConfig",
    "WandbConfig",
    "XAIChatConfig",
    "ZAIChatConfig",
    "CohereChatConfig",
    "CohereV2ChatConfig",
    "OpenAIImageVariationConfig",
    "TopazImageVariationConfig",
    "VertexAIAnthropicConfig",
    "VertexAIAi21Config",
    "VertexAILlama3Config",
    "VertexAIMistralConfig",
)

_BEDROCK_CLASSES: Final[tuple[str, ...]] = (
    "AmazonAI21Config",
    "AmazonAnthropicClaudeConfig",
    "AmazonBedrockOpenAIConfig",
    "AmazonBedrockRuntimeChatCompletionsConfig",
    "AmazonCohereConfig",
    "AmazonConverseConfig",
    "AmazonDeepSeekR1Config",
    "AmazonInvokeConfig",
    "AmazonInvokeNovaConfig",
    "AmazonLlamaConfig",
    "AmazonMistralConfig",
    "AmazonMoonshotConfig",
    "AmazonQwen2Config",
    "AmazonQwen3Config",
    "AmazonTitanConfig",
    "AmazonTwelveLabsPegasusConfig",
    "BedrockClaudePlatformConfig",
)

_MODULE_CLASSES: Final[tuple[tuple[str, str], ...]] = (
    ("litellm.llms.openai.chat.gpt_transformation", "OpenAIUnknownModelConfig"),
    (
        "litellm.llms.vertex_ai.vertex_ai_partner_models.gpt_oss.transformation",
        "VertexAIGPTOSSTransformation",
    ),
    ("litellm.llms.azure_ai.azure_model_router.transformation", "AzureModelRouterConfig"),
    ("litellm.llms.azure_ai.anthropic.transformation", "AzureAnthropicConfig"),
    ("litellm.llms.azure_ai.chat.transformation", "AzureAIStudioConfig"),
    ("litellm.llms.sail.chat.transformation", "SailChatConfig"),
    ("litellm.llms.langgraph.chat.transformation", "LangGraphConfig"),
    ("litellm.llms.langflow.chat.transformation", "LangFlowConfig"),
    ("litellm.llms.bedrock.chat.agentcore.transformation", "AmazonAgentCoreConfig"),
    ("litellm.llms.bedrock.chat.invoke_agent.transformation", "AmazonInvokeAgentConfig"),
    ("litellm.llms.bedrock.chat.mantle.transformation", "AmazonMantleConfig"),
)

_MIRRORS: Final[tuple[tuple[str, type], ...]] = (
    ("OpenAIGPT5Config", OpenAIGPT5Config),
    ("AzureOpenAIConfig", AzureOpenAIConfig),
    ("AzureOpenAIGPT5Config", AzureOpenAIGPT5Config),
    ("AzureOpenAIO1Config", AzureOpenAIO1Config),
)


class Factory:
    def __init__(self, name: str) -> None:
        self.name = name

    def __call__(self) -> object:
        return cast(object, type(self.name, (), {})())


def _json_factory(_config: object) -> Factory:
    return Factory("JSONProviderConfig")


class AnthropicFactory(Factory):
    @staticmethod
    def get_legacy_anthropic_model_names() -> list[str]:
        names: list[str] = AmazonAnthropicConfig.get_legacy_anthropic_model_names()  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        return names


class Watcher:
    def __init__(self, method: Callable[..., object]) -> None:
        self.method = method
        self.arguments: list[str] = []

    def __call__(self, *args: object, **kwargs: object) -> object:
        self.arguments = [value for value in (*args, *kwargs.values()) if isinstance(value, str)]
        return self.method(*args, **kwargs)


class Registry:
    def __init__(self, entries: list[dict[str, Json]]) -> None:
        self.existing: set[str] = set()
        self.records: dict[str, SimpleProviderConfig] = {}
        for entry in entries:
            slug = _text(entry, "provider")
            if not _bool(entry, "exists"):
                continue
            self.existing.add(slug)
            if _optional_bool(entry, "record"):
                self.records[slug] = SimpleProviderConfig(
                    slug, {"base_url": "https://example.test", "api_key_env": "BETTERLITE_TEST_KEY"}
                )

    def exists_provider(self, slug: str) -> bool:
        return slug in self.existing

    def record(self, slug: str) -> SimpleProviderConfig | None:
        return self.records.get(slug)


class Fixture:
    def __init__(self, watchers: list[Watcher]) -> None:
        self.watchers = watchers

    def clear(self) -> None:
        for watcher in self.watchers:
            watcher.arguments = []

    def arguments(self) -> list[str]:
        for watcher in self.watchers:
            if watcher.arguments:
                return list(watcher.arguments)
        return []


def _object(fields: dict[str, Json], name: str) -> dict[str, Json]:
    value = fields.get(name, {})
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def _objects(fields: dict[str, Json], name: str) -> list[dict[str, Json]]:
    value = fields.get(name, [])
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [cast(dict[str, Json], item) for item in value]


def _text(fields: dict[str, Json], name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _optional_text(fields: dict[str, Json], name: str) -> str | None:
    value = fields.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _bool(fields: dict[str, Json], name: str) -> bool:
    value = fields.get(name)
    if isinstance(value, bool):
        return value
    raise TypeError(f"{name} must be a boolean")


def _optional_bool(fields: dict[str, Json], name: str) -> bool:
    if name not in fields:
        return False
    return _bool(fields, name)


def _provider(fields: dict[str, Json]) -> LlmProviders:
    return LlmProviders(_text(fields, "provider"))


def _strings(fields: dict[str, Json], name: str) -> list[str]:
    value = fields.get(name, [])
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [_text({"item": item}, "item") for item in value]


def _watch(stack: ExitStack, name: str, method: Callable[..., object]) -> Watcher:
    watcher = Watcher(method)
    stack.enter_context(patch.object(ProviderConfigManager, name, watcher))
    return watcher


def _install(stack: ExitStack, input_value: dict[str, Json]) -> Fixture:
    catalog = _object(input_value, "catalog")
    vertex = _object(input_value, "vertex")
    chat_models = set(_strings(catalog, "chat_models"))
    text_models = set(_strings(catalog, "text_models"))
    stack.enter_context(patch.object(litellm, "open_ai_chat_completion_models", chat_models))
    stack.enter_context(patch.object(litellm, "open_ai_text_completion_models", text_models))
    mistral_models = set(_strings(vertex, "mistral_models"))
    ai21_models = set(_strings(vertex, "ai21_models"))
    stack.enter_context(patch.object(litellm, "vertex_mistral_models", mistral_models))
    stack.enter_context(patch.object(litellm, "vertex_ai_ai21_models", ai21_models))
    stack.enter_context(patch.object(litellm, "model_cost", cast(dict[str, Json], {})))
    stack.enter_context(patch.object(litellm, "bedrock_converse_models", set[str]()))
    for symbol in (*_CLASSES, *_BEDROCK_CLASSES):
        stack.enter_context(patch.object(litellm, symbol, Factory(symbol)))
    stack.enter_context(
        patch.object(litellm, "AmazonAnthropicConfig", AnthropicFactory("AmazonAnthropicConfig"))
    )
    for symbol, base in _MIRRORS:
        mirror = cast(type, type(symbol, (base,), {}))
        stack.enter_context(patch.object(litellm, symbol, mirror))
    for module_path, attribute in _MODULE_CLASSES:
        stack.enter_context(patch.object(import_module(module_path), attribute, Factory(attribute)))
    registry = Registry(_objects(input_value, "providers"))
    stack.enter_context(patch.object(JSONProviderRegistry, "exists", registry.exists_provider))
    stack.enter_context(patch.object(JSONProviderRegistry, "get", registry.record))
    stack.enter_context(
        patch.object(
            dynamic_config,
            "create_config_class",
            _json_factory,
        )
    )
    watchers = [
        _watch(stack, "_get_azure_ai_config", ProviderConfigManager._get_azure_ai_config),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        _watch(stack, "_get_vertex_ai_config", ProviderConfigManager._get_vertex_ai_config),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        _watch(stack, "_get_bedrock_config", ProviderConfigManager._get_bedrock_config),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        _watch(stack, "_get_cohere_config", ProviderConfigManager._get_cohere_config),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    ]
    return Fixture(watchers)


def _parts(fixture: Fixture, selected: object) -> dict[str, Json]:
    if selected is litellm.openaiOSeriesConfig:
        return {"class": "OpenAIOSeriesConfig", "arguments": [], "constructed": False}
    name = type(selected).__name__
    return {"class": name, "arguments": cast(Json, fixture.arguments()), "constructed": True}


def _report(fixture: Fixture, selected: object | None) -> Json:
    if selected is None:
        return None
    return cast(Json, _parts(fixture, selected))


def _chat_report(fixture: Fixture, call: dict[str, Json]) -> Json:
    fixture.clear()
    selected = ProviderConfigManager.get_provider_chat_config(
        model=_text(call, "model"),
        provider=_provider(call),
        base_model=_optional_text(call, "base_model"),
    )
    return _report(fixture, selected)


def _foundry_report(fixture: Fixture, call: dict[str, Json]) -> Json:
    fixture.clear()
    model = _text(call, "model")
    route = AzureFoundryModelInfo.get_azure_ai_route(model)
    selected = ProviderConfigManager.get_provider_chat_config(
        model=model, provider=LlmProviders.AZURE_AI
    )
    if selected is None:
        return cast(Json, {"route": route, "class": None, "arguments": [], "constructed": False})
    return cast(Json, {"route": route, **_parts(fixture, selected)})


def _json_report(fixture: Fixture, call: dict[str, Json]) -> Json:
    fixture.clear()
    provider = _provider(call)
    try:
        selected = ProviderConfigManager.get_provider_chat_config(
            model=_text(call, "model"), provider=provider
        )
    except ValueError:
        return cast(Json, {"outcome": "error", "provider": provider.value})
    if selected is None:
        return cast(Json, {"outcome": "absent"})
    return cast(Json, {"outcome": "selected", **_parts(fixture, selected)})


def _image_report(fixture: Fixture, call: dict[str, Json]) -> Json:
    fixture.clear()
    selected = ProviderConfigManager.get_provider_image_variation_config(
        model=_text(call, "model"), provider=_provider(call)
    )
    return _report(fixture, selected)


_CHAT_FUNCTIONS: Final[frozenset[str]] = frozenset(
    {
        "openai_family_selection",
        "azure_detection_selection",
        "factory_table",
        "vertex_selection",
        "cohere_and_bedrock_selection",
    }
)


def run(input_value: dict[str, Json]) -> Json:
    function = _text(input_value, "function")
    calls = _objects(input_value, "calls")
    with ExitStack() as stack:
        fixture = _install(stack, input_value)
        if function in _CHAT_FUNCTIONS:
            return cast(Json, [_chat_report(fixture, call) for call in calls])
        if function == "foundry_selection":
            return cast(Json, [_foundry_report(fixture, call) for call in calls])
        if function == "json_fallback_precedence":
            return cast(Json, [_json_report(fixture, call) for call in calls])
        if function == "image_variation_config_selection":
            return cast(Json, [_image_report(fixture, call) for call in calls])
        raise ValueError(f"unknown chat_config_selection function: {function}")
