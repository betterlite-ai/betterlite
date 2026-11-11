import io
from collections.abc import Callable, Generator
from contextlib import contextmanager, redirect_stdout
from typing import NotRequired, TypeAlias, TypedDict, cast
from unittest import mock

import litellm
import litellm.utils as litellm_utils
from litellm.llms.lemonade.chat.transformation import LemonadeChatConfig
from litellm.llms.ollama.common_utils import OllamaModelInfo
from litellm.utils import (  # pyright: ignore[reportPrivateUsage]
    _supports_factory,  # pyright: ignore[reportPrivateUsage]
    declared_value_factory,
    is_explicitly_disabled_factory,
    supports_function_calling,
    supports_mid_conversation_system,
    supports_native_streaming,
    supports_native_structured_output,
    supports_none_reasoning_effort,
    supports_parallel_function_calling,
    supports_prompt_caching,
    supports_reasoning,
    supports_response_schema,
    supports_system_messages,
    supports_tool_choice,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
ModelCost: TypeAlias = dict[str, Json]
Handler: TypeAlias = Callable[[str, "str | None", "str | None"], Json]

add_known_models = cast(Callable[[ModelCost], None], litellm.add_known_models)


class Step(TypedDict):
    function: str
    model: str
    custom_llm_provider: NotRequired[str | None]
    key: NotRequired[str]


class Fixture(TypedDict):
    model_cost: NotRequired[dict[str, object]]


class Input(TypedDict):
    fixture: NotRequired[Fixture]
    steps: list[Step]


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def object_list(value: Json, name: str) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(item, name) for item in value]


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


class FakeClient:
    def post(self, *_args: object, **_kwargs: object) -> object:
        raise AssertionError("a capability predicate must not call the provider")

    def get(self, *_args: object, **_kwargs: object) -> object:
        raise AssertionError("a capability predicate must not call the provider")


def refusing_dynamic(
    _self: object,
    _model: str,
    _api_base: str | None = None,
    _api_key: str | None = None,
) -> None:
    return None


def model_cost_of(fixture: Fixture) -> ModelCost:
    fields = cast(Object, fixture)
    declared = fields.get("model_cost")
    if declared is None:
        return {}
    return dict(object_value(declared, "model_cost").items())


@contextmanager
def installed(fixture: Fixture) -> Generator[None]:
    costs = model_cost_of(fixture)
    with (
        mock.patch.object(litellm, "model_cost", costs),
        mock.patch.object(OllamaModelInfo, "get_model_info", refusing_dynamic),
        mock.patch.object(LemonadeChatConfig, "get_model_info", refusing_dynamic),
        mock.patch.object(litellm, "module_level_client", FakeClient()),
    ):
        add_known_models(costs)
        litellm_utils._model_cost_lowercase_map = None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        yield


def capability_supports(model: str, provider: str | None, key: str | None) -> Json:
    if key is None:
        raise ValueError("supports_factory needs a key")
    return _supports_factory(model=model, custom_llm_provider=provider, key=key)


def capability_declared(model: str, provider: str | None, key: str | None) -> Json:
    if key is None:
        raise ValueError("declared_value_factory needs a key")
    return declared_value_factory(model=model, custom_llm_provider=provider, key=key)


def capability_disabled(model: str, provider: str | None, key: str | None) -> Json:
    if key is None:
        raise ValueError("is_explicitly_disabled_factory needs a key")
    return is_explicitly_disabled_factory(model=model, custom_llm_provider=provider, key=key)


def capability_native_streaming(model: str, provider: str | None, _key: str | None) -> Json:
    return supports_native_streaming(model, provider)


def capability_response_schema(model: str, provider: str | None, _key: str | None) -> Json:
    return supports_response_schema(model, provider)


def named(handler: Callable[[str, str | None], bool]) -> Handler:
    def call(model: str, provider: str | None, _key: str | None) -> Json:
        return handler(model, provider)

    return call


HANDLERS: dict[str, Handler] = {
    "supports_factory": capability_supports,
    "declared_value_factory": capability_declared,
    "is_explicitly_disabled_factory": capability_disabled,
    "supports_native_streaming": capability_native_streaming,
    "supports_response_schema": capability_response_schema,
    "supports_system_messages": named(supports_system_messages),
    "supports_parallel_function_calling": named(supports_parallel_function_calling),
    "supports_function_calling": named(supports_function_calling),
    "supports_tool_choice": named(supports_tool_choice),
    "supports_prompt_caching": named(supports_prompt_caching),
    "supports_reasoning": named(supports_reasoning),
    "supports_none_reasoning_effort": named(supports_none_reasoning_effort),
    "supports_mid_conversation_system": named(supports_mid_conversation_system),
    "supports_native_structured_output": named(supports_native_structured_output),
}


def run_step(step: Object) -> Json:
    function = string_value(step.get("function"), "function")
    handler = HANDLERS.get(function)
    if handler is None:
        raise ValueError(f"unknown capability function: {function}")
    declared_key = step.get("key")
    key = None if declared_key is None else string_value(declared_key, "key")
    return handler(
        string_value(step.get("model"), "model"),
        optional_text(step, "custom_llm_provider"),
        key,
    )


def run(input_value: dict[str, Json]) -> Json:
    fixture = object_value(input_value.get("fixture", {}), "fixture")
    steps = object_list(input_value.get("steps"), "steps")
    with installed(cast(Fixture, fixture)), redirect_stdout(io.StringIO()):
        return [run_step(step) for step in steps]
