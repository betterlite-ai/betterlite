import logging
from collections.abc import Callable
from typing import Final, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.anthropic.pass_through.messages.transformation import (
    AnthropicMessagesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JsonObject: TypeAlias = dict[str, JsonValue]
NO_PARAMS: Final[JsonObject] = {}
RESULT: Final = TypeAdapter(JsonObject)
WARNINGS: Final = TypeAdapter(list[JsonValue])


class Call(TypedDict):
    model: str
    custom_llm_provider: str | None
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    strips_billing_metadata: bool


class Input(TypedDict):
    model_cost: dict[str, dict[str, JsonValue]]
    packaged: dict[str, JsonValue]
    calls: list[Call]


Transform: TypeAlias = Callable[
    [str, list[dict[str, JsonValue]], JsonObject, GenericLiteLLMParams, JsonObject],
    JsonObject,
]


class Config(Protocol):
    def __getattr__(self, name: str) -> Transform: ...


class Watch(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.warnings: list[str] = []
        self.transformation = False

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if record.levelno == logging.WARNING:
            self.warnings.append(message)
        elif record.levelno == logging.DEBUG and message.startswith("TRANSFORMATION DEBUG"):
            self.transformation = True


class ProviderConfig(AnthropicMessagesConfig):
    def __getattr__(self, name: str) -> Transform:
        raise AttributeError(name)

    @staticmethod
    def for_call(provider: str, *, strips_billing_metadata: bool) -> Config:
        return ProviderConfig(provider, strips_billing_metadata=strips_billing_metadata)

    def __init__(self, provider: str, *, strips_billing_metadata: bool) -> None:
        self.provider = provider
        self.strips_billing_metadata = strips_billing_metadata

    @property
    @override
    def custom_llm_provider(self) -> str:
        return self.provider

    @override
    def should_strip_billing_metadata(self) -> bool:
        return self.strips_billing_metadata


def outcome(request: JsonObject, watch: Watch) -> JsonObject:
    return {
        "request": request,
        "warnings": WARNINGS.validate_python(watch.warnings),
        "debug": watch.transformation,
    }


def call_result(call: Call) -> dict[str, JsonValue]:
    provider = (
        "anthropic"
        if call["custom_llm_provider"] is None
        else call["custom_llm_provider"]
    )
    config = ProviderConfig.for_call(
        provider, strips_billing_metadata=call["strips_billing_metadata"]
    )
    watch = Watch()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(watch)
    try:
        transformed = config.transform_anthropic_messages_request(
            call["model"],
            call["messages"],
            dict(call["optional_params"]),
            GenericLiteLLMParams(),
            NO_PARAMS,
        )
    finally:
        logger.removeHandler(watch)
    return outcome(RESULT.validate_python(transformed), watch)


def run(argument: Input) -> JsonValue:
    litellm.register_model(argument["model_cost"])
    return {"calls": [call_result(call) for call in argument["calls"]]}
