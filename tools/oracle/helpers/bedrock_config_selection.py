from contextlib import ExitStack
from importlib import import_module
from typing import Final, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.bedrock.chat.invoke_transformations.anthropic_claude2_transformation import (
    AmazonAnthropicConfig,
)
from litellm.llms.bedrock.common_utils import BedrockModelInfo, get_bedrock_chat_config

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_ARGUMENTS: Final[dict[str, list[Json]]] = {}

_PATCHES: Final[tuple[tuple[str, str], ...]] = (
    ("litellm", "AmazonAnthropicClaudeConfig"),
    ("litellm", "AmazonAnthropicClaudeMessagesConfig"),
    ("litellm", "AmazonAnthropicConfig"),
    ("litellm", "AmazonAI21Config"),
    ("litellm", "AmazonBedrockOpenAIConfig"),
    ("litellm", "AmazonBedrockRuntimeChatCompletionsConfig"),
    ("litellm", "AmazonCohereConfig"),
    ("litellm", "AmazonConverseConfig"),
    ("litellm", "AmazonDeepSeekR1Config"),
    ("litellm", "AmazonInvokeConfig"),
    ("litellm", "AmazonInvokeNovaConfig"),
    ("litellm", "AmazonLlamaConfig"),
    ("litellm", "AmazonMistralConfig"),
    ("litellm", "AmazonMoonshotConfig"),
    ("litellm", "AmazonQwen2Config"),
    ("litellm", "AmazonQwen3Config"),
    ("litellm", "AmazonTitanConfig"),
    ("litellm", "AmazonTwelveLabsPegasusConfig"),
    ("litellm", "BedrockClaudePlatformConfig"),
    ("litellm", "BedrockClaudePlatformMessagesConfig"),
    ("litellm.llms.bedrock.chat.agentcore.transformation", "AmazonAgentCoreConfig"),
    ("litellm.llms.bedrock.chat.invoke_agent.transformation", "AmazonInvokeAgentConfig"),
    ("litellm.llms.bedrock.chat.mantle.transformation", "AmazonMantleConfig"),
    ("litellm.llms.bedrock.messages.mantle_transformation", "AmazonMantleMessagesConfig"),
)


class Factory:
    def __init__(self, name: str) -> None:
        self.name = name

    def __call__(self, *args: object, **kwargs: object) -> object:
        if args or kwargs:
            raise TypeError(f"{self.name} was constructed with arguments")
        _ARGUMENTS[self.name] = []
        return cast(object, type(self.name, (), {})())


class AnthropicFactory(Factory):
    @staticmethod
    def get_legacy_anthropic_model_names() -> list[str]:
        names: list[str] = AmazonAnthropicConfig.get_legacy_anthropic_model_names()  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        return names


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def strings(value: Json, name: str) -> list[str]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [string_value(item, name) for item in value]


def install(stack: ExitStack) -> None:
    for module_path, attribute in _PATCHES:
        target = import_module(module_path)
        replacement = (
            AnthropicFactory(attribute)
            if attribute == "AmazonAnthropicConfig"
            else Factory(attribute)
        )
        stack.enter_context(patch.object(target, attribute, replacement))
    # the real model map routes far more models to Converse than the case fixtures ask for
    cast("dict[str, Json]", litellm.model_cost).clear()
    cast("set[str]", litellm.bedrock_converse_models).clear()


def report(config: object | None) -> Json:
    if config is None:
        return None
    name = type(config).__name__
    arguments = _ARGUMENTS.pop(name, None)
    if arguments is None:
        raise TypeError(f"{name} was not constructed through the fixture")
    return cast(Json, {"class": name, "arguments": arguments})


def chat_report(model: str) -> Json:
    selected = get_bedrock_chat_config(model=model)  # pyright: ignore[reportUnknownVariableType]
    return report(cast(object, selected))


def native_report(model: str) -> Json:
    selected = BedrockModelInfo.get_bedrock_provider_config_for_messages_api(model)  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    return report(cast(object, selected))


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    if function == "legacy_anthropic_names":
        names: list[str] = AmazonAnthropicConfig.get_legacy_anthropic_model_names()  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        return cast(Json, names)
    models = strings(input_value.get("models"), "models")
    with ExitStack() as stack:
        install(stack)
        if function == "chat_config_selector":
            return cast(Json, [chat_report(model) for model in models])
        if function == "native_messages_route":
            return cast(Json, [native_report(model) for model in models])
        if function == "selection_without_construction":
            return cast(
                Json,
                [
                    {"chat": chat_report(model), "messages": native_report(model)}
                    for model in models
                ],
            )
    raise ValueError(f"unknown bedrock_config_selection function: {function}")
