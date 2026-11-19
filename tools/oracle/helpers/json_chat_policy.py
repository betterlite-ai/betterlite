import os
from typing import Any, Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.openai_like.dynamic_config import create_config_class
from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig
from litellm.litellm_core_utils.prompt_templates.common_utils import (
    handle_messages_with_content_list_to_str_conversion,
)
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        drop_params: bool,
    ) -> JsonObject: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: JsonObject,
        litellm_params: JsonObject,
        stream: bool | None = None,
    ) -> str: ...


class ChatConfigClass(Protocol):
    def __call__(self) -> ChatConfig: ...


class ChatConfigFactory(Protocol):
    def __call__(self, provider: SimpleProviderConfig) -> ChatConfigClass: ...


# the documented entry point for a generated JSON provider config, typed through the protocol above
FACTORY: Final[ChatConfigFactory] = create_config_class
MESSAGES: Final[TypeAdapter[list[JsonValue]]] = TypeAdapter(list[JsonValue])


class IdentityCall(TypedDict):
    kind: Literal["identity"]
    model: str
    environment: NotRequired[dict[str, str]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]


class SupportedCall(TypedDict):
    kind: Literal["supported"]
    model: str
    environment: NotRequired[dict[str, str]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]


class ParamsCall(TypedDict):
    kind: Literal["params"]
    model: str
    environment: NotRequired[dict[str, str]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    descriptor: NotRequired[JsonObject | None]
    non_default: JsonObject
    optional: JsonObject


class UrlCall(TypedDict):
    kind: Literal["url"]
    model: str
    environment: NotRequired[dict[str, str]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]


class MessagesCall(TypedDict):
    kind: Literal["messages"]
    model: str
    environment: NotRequired[dict[str, str]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    # pydantic cannot validate LiteLLM's message union without materializing lazy iterables,
    # so the case's plain message dicts reach AllMessageValues through Any
    messages: list[Any]


Call: TypeAlias = IdentityCall | SupportedCall | ParamsCall | UrlCall | MessagesCall


class Input(TypedDict):
    calls: list[Call]


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)


def stripped_model(model: str) -> str:
    return model.split("/", 1)[1]


def provider_for(model: str, descriptor: JsonObject | None) -> SimpleProviderConfig:
    slug = model.split("/", 1)[0]
    if descriptor is not None:
        return SimpleProviderConfig(slug=slug, data=descriptor)
    provider = JSONProviderRegistry.get(slug)
    if provider is None:
        raise ValueError(f"{slug} is not a JSON provider")
    return provider


def chat_config(provider: SimpleProviderConfig) -> ChatConfig:
    return FACTORY(provider)()


def identity_value(call: IdentityCall) -> JsonValue:
    resolved: tuple[str, str, str | None, str | None] = litellm.get_llm_provider(
        model=call["model"], api_base=call.get("api_base"), api_key=call.get("api_key")
    )
    model, provider, key, base = resolved
    result: dict[str, JsonValue] = {
        "model": model,
        "provider": provider,
        "base": base,
        "key": key,
    }
    return result


def supported_value(call: SupportedCall) -> JsonValue:
    config = chat_config(provider_for(call["model"], None))
    names = config.get_supported_openai_params(stripped_model(call["model"]))
    supported: list[JsonValue] = [name for name in names]
    return supported


def params_value(call: ParamsCall) -> JsonValue:
    config = chat_config(provider_for(call["model"], call.get("descriptor")))
    mapped = config.map_openai_params(
        non_default_params=dict(call["non_default"]),
        optional_params=dict(call["optional"]),
        model=stripped_model(call["model"]),
        drop_params=False,
    )
    return mapped


def url_value(call: UrlCall) -> JsonValue:
    config = chat_config(provider_for(call["model"], None))
    url = config.get_complete_url(
        api_base=call.get("api_base"),
        api_key=None,
        model=stripped_model(call["model"]),
        optional_params={},
        litellm_params={},
    )
    return url


def messages_value(call: MessagesCall) -> JsonValue:
    converted = handle_messages_with_content_list_to_str_conversion(call["messages"])
    return MESSAGES.validate_python(converted)


def call_value(call: Call) -> JsonValue:
    if call["kind"] == "identity":
        return identity_value(call)
    if call["kind"] == "supported":
        return supported_value(call)
    if call["kind"] == "params":
        return params_value(call)
    if call["kind"] == "url":
        return url_value(call)
    return messages_value(call)


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    results: list[JsonValue] = []
    for call in parsed["calls"]:
        os.environ.clear()
        environment = call.get("environment")
        if environment is not None:
            os.environ.update(environment)
        results.append(call_value(call))
    return results
