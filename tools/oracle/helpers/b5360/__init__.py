from collections.abc import Callable
from contextlib import ExitStack
from importlib import import_module
from typing import Final, TypeAlias, cast
from unittest.mock import patch

import litellm
import litellm.utils
from litellm.harness.types import Harness as LiteLLMHarness
from litellm.llms.azure_ai import common_utils as azure_ai_common
from litellm.llms.bedrock.common_utils import BedrockModelInfo
from litellm.llms.bedrock_mantle import common_utils as mantle_common
from litellm.llms.github_copilot.responses import transformation as copilot_responses
from litellm.llms.vertex_ai.text_to_speech import transformation as vertex_speech
from litellm.types.utils import LlmProviders, SearchProviders
from litellm.utils import ProviderConfigManager

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_LLMS: Final = "litellm.llms."
_ARGUMENTS: Final[dict[str, Json]] = {}

_FACTORIES: Final[dict[str, str]] = {
    "openai.image_generation": "OpenAIImageGenerationConfig",
    "azure.image_generation": "AzureImageGenerationConfig",
    "azure_ai.image_generation": "AzureAIImageGenerationConfig",
    "xinference.image_generation": "XinferenceImageGenerationConfig",
    "recraft.image_generation": "RecraftImageGenerationConfig",
    "aiml.image_generation": "AimlImageGenerationConfig",
    "cometapi.image_generation": "CometAPIImageGenerationConfig",
    "gemini.image_generation": "GeminiImageGenerationConfig",
    "fal_ai.image_generation": "FalAIImageGenerationConfig",
    "stability.image_generation": "StabilityImageGenerationConfig",
    "runwayml.image_generation": "RunwayMLImageGenerationConfig",
    "black_forest_labs.image_generation": "BlackForestLabsImageGenerationConfig",
    "vertex_ai.image_generation": "VertexAIImageGenerationConfig",
    "openrouter.image_generation": "OpenRouterImageGenerationConfig",
    "modelscope.image_generation": "ModelScopeImageGenerationConfig",
    "openai.image_edit": "OpenAIImageEditConfig",
    "hosted_vllm.image_edit": "HostedVLLMImageEditConfig",
    "fal_ai.image_edit": "FalAIImageEditConfig",
    "azure_ai.image_edit": "AzureAIImageEditConfig",
    "gemini.image_edit": "GeminiImageEditConfig",
    "vertex_ai.image_edit": "VertexAIImageEditConfig",
    "stability.image_edit": "StabilityImageEditConfig",
    "openrouter.image_edit": "OpenRouterImageEditConfig",
}


class Factory:
    def __init__(self, label: str) -> None:
        self.label = label

    def __call__(self, model: str) -> object:
        _ARGUMENTS[self.label] = model
        return cast(object, type(self.label, (), {})())


class MantleFactory:
    def __call__(self, _signer: object = None, *, use_openai_path: bool = True) -> object:
        _ARGUMENTS["BedrockMantleResponsesAPIConfig"] = use_openai_path
        return cast(object, type("BedrockMantleResponsesAPIConfig", (), {})())


class Facts:
    def __init__(self) -> None:
        self.supports_reasoning = False
        self.native_responses = False
        self.github_copilot = False
        self.mantle_supported = False
        self.use_openai_path = False
        self.is_lyria = False

    def update(self, call: dict[str, Json]) -> None:
        facts = _object(call, "facts")
        azure = _object(facts, "azure")
        mantle = _object(facts, "mantle")
        self.supports_reasoning = _bool(azure, "supports_reasoning")
        self.native_responses = _bool(azure, "native_responses")
        self.github_copilot = _bool(facts, "github_copilot_responses")
        self.mantle_supported = _bool(mantle, "supported")
        self.use_openai_path = _bool(mantle, "use_openai_path")
        self.is_lyria = _bool(call, "is_lyria")


_FACTS: Final = Facts()


def _object(fields: dict[str, Json], name: str) -> dict[str, Json]:
    value = fields.get(name, {})
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def _objects(fields: dict[str, Json], name: str) -> list[dict[str, Json]]:
    value = fields.get(name)
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
    value = fields.get(name, False)
    if isinstance(value, bool):
        return value
    raise TypeError(f"{name} must be a boolean")


def _provider(fields: dict[str, Json]) -> LlmProviders:
    return LlmProviders(_text(fields, "provider"))


def _optional_provider(fields: dict[str, Json]) -> LlmProviders | None:
    slug = _optional_text(fields, "provider")
    return None if slug is None else LlmProviders(slug)


def _patch_factories(stack: ExitStack) -> None:
    for module_suffix, label in _FACTORIES.items():
        provider, family = module_suffix.split(".", 1)
        module = import_module(_LLMS + module_suffix)
        attribute = f"get_{provider}_{family}_config"
        stack.enter_context(patch.object(module, attribute, Factory(label)))
    stack.enter_context(
        patch.object(
            import_module(_LLMS + "dashscope.common_utils"),
            "get_dashscope_family_image_generation_config",
            Factory("DashscopeFamilyImageGenerationConfig"),
        )
    )
    bedrock_edit = import_module(
        _LLMS + "bedrock.image_edit.amazon_nova_canvas_image_edit_transformation"
    )
    stack.enter_context(
        patch.object(
            bedrock_edit,
            "get_bedrock_image_edit_config_for_model",
            Factory("BedrockImageEditConfig"),
        )
    )


def _supports_reasoning(*_args: Json, **_kwargs: Json) -> bool:
    return _FACTS.supports_reasoning


def _native_responses(*_args: Json, **_kwargs: Json) -> bool:
    return _FACTS.native_responses


def _github_copilot(*_args: Json, **_kwargs: Json) -> bool:
    return _FACTS.github_copilot


def _mantle_responses(*_args: Json, **_kwargs: Json) -> bool:
    return _FACTS.mantle_supported


def _mantle_segment(*_args: Json, **_kwargs: Json) -> str:
    return "openai/v1" if _FACTS.use_openai_path else "v1"


def _lyria(*_args: Json, **_kwargs: Json) -> bool:
    return _FACTS.is_lyria


def _vertex_lyria_class() -> type:
    return cast(type, vertex_speech.VertexAILyriaTextToSpeechConfig)


def _install(stack: ExitStack, input_value: dict[str, Json]) -> None:
    live = _object(input_value, "model_cost")
    bundled = _object(input_value, "bundled_model_cost")
    stack.enter_context(patch.object(litellm, "model_cost", live))
    stack.enter_context(patch.object(litellm.utils, "_get_bundled_model_cost_map", lambda: bundled))
    patches: Final[list[tuple[object, str, object]]] = [
        (litellm.utils, "supports_reasoning", _supports_reasoning),
        (azure_ai_common, "azure_ai_supports_native_responses", _native_responses),
        (copilot_responses, "github_copilot_supports_responses_api", _github_copilot),
        (mantle_common, "mantle_supports_responses", _mantle_responses),
        (mantle_common, "mantle_base_segment", _mantle_segment),
        (_vertex_lyria_class(), "is_lyria_model", _lyria),
        (
            BedrockModelInfo,
            "get_bedrock_provider_config_for_messages_api",
            Factory("BedrockMessagesConfig"),
        ),
        (
            litellm.BedrockOpenAIResponsesConfig,
            "for_model",
            Factory("BedrockOpenAIResponsesConfig"),
        ),
        (litellm, "BedrockMantleResponsesAPIConfig", MantleFactory()),
        (
            litellm,
            "GithubCopilotResponsesAPIConfig",
            lambda: cast(object, type("GithubCopilotResponsesAPIConfig", (), {})()),
        ),
    ]
    for target, attribute, replacement in patches:
        stack.enter_context(patch.object(target, attribute, replacement))
    _patch_factories(stack)


_MANAGER: Final = ProviderConfigManager


def _model_and_provider(
    method: Callable[..., object | None],
) -> Callable[[dict[str, Json]], object | None]:
    return lambda call: method(model=_text(call, "model"), provider=_provider(call))


def _provider_only(
    method: Callable[..., object | None],
) -> Callable[[dict[str, Json]], object | None]:
    return lambda call: method(provider=_provider(call))


_SELECTORS: Final[dict[str, Callable[[dict[str, Json]], object | None]]] = {
    "transcription": _model_and_provider(_MANAGER.get_provider_audio_transcription_config),
    "image_generation": _model_and_provider(_MANAGER.get_provider_image_generation_config),
    "image_edit": _model_and_provider(_MANAGER.get_provider_image_edit_config),
    "text_to_speech": _model_and_provider(_MANAGER.get_provider_text_to_speech_config),
    "messages": _model_and_provider(_MANAGER.get_provider_anthropic_messages_config),
    "passthrough": _model_and_provider(_MANAGER.get_provider_passthrough_config),
    "batches": _model_and_provider(_MANAGER.get_provider_batches_config),
    "realtime": _model_and_provider(_MANAGER.get_provider_realtime_config),
    "realtime_http": _model_and_provider(_MANAGER.get_provider_realtime_http_config),
    "vector_store": _provider_only(_MANAGER.get_provider_vector_store_config),
    "vector_store_files": _provider_only(_MANAGER.get_provider_vector_store_files_config),
    "skills": _provider_only(_MANAGER.get_provider_skills_api_config),
    "evals": _provider_only(_MANAGER.get_provider_evals_api_config),
    "responses": lambda call: _MANAGER._get_python_responses_api_config(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        provider=_optional_provider(call), model=_optional_text(call, "model")
    ),
    "vector_stores": lambda call: _MANAGER.get_provider_vector_stores_config(
        provider=_provider(call), api_type=_optional_text(call, "api_type")
    ),
    "search": lambda call: _MANAGER.get_provider_search_config(
        provider=SearchProviders(_text(call, "search_provider"))
    ),
    "harness": lambda call: cast(
        object | None,
        _MANAGER.get_provider_harness_config(  # pyright: ignore[reportUnknownMemberType]
            harness=LiteLLMHarness(_text(call, "harness"))
        ),
    ),
}


def _config(selector: str, call: dict[str, Json]) -> object | None:
    handler = _SELECTORS.get(selector)
    if handler is None:
        raise ValueError(f"unknown b5360 selector: {selector}")
    return handler(call)


def _report(config: object | None) -> Json:
    if config is None:
        return None
    name = type(config).__name__
    return {"class": name, "arguments": _ARGUMENTS.pop(name, [])}


def run(input_value: dict[str, Json]) -> Json:
    results: list[Json] = []
    with ExitStack() as stack:
        _install(stack, input_value)
        for request in _objects(input_value, "requests"):
            selector = _text(request, "selector")
            batch: list[Json] = []
            for call in _objects(request, "calls"):
                _ARGUMENTS.clear()
                _FACTS.update(call)
                batch.append(_report(_config(selector, call)))
            results.append(batch)
    return results
