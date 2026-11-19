import os
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.openai_like.dynamic_config import create_config_class
from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig
from litellm.litellm_core_utils.prompt_templates.common_utils import (
    handle_messages_with_content_list_to_str_conversion as collapse_messages,
)
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
MESSAGES: Final[TypeAdapter[list[JsonValue]]] = TypeAdapter(list[JsonValue])


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[JsonValue]: ...

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


class ChatFactory(Protocol):
    def __call__(self, provider: SimpleProviderConfig) -> ChatConfigClass: ...


class CollapseMessages(Protocol):
    def __call__(self, messages: list[JsonObject]) -> list[JsonObject]: ...


# the documented entry point for a generated JSON provider config, typed through the protocol above
FACTORY: Final[ChatFactory] = create_config_class
# AllMessageValues carries Any, so the case's plain dicts go through a protocol instead
COLLAPSE: Final[CollapseMessages] = cast(CollapseMessages, collapse_messages)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/common_utils.py#L82

CallKind: TypeAlias = Literal["identity", "supported", "params", "url", "messages"]


class Call(TypedDict):
    kind: CallKind
    model: str
    environment: NotRequired[dict[str, str]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    descriptor: NotRequired[JsonObject | None]
    non_default: NotRequired[JsonObject]
    optional: NotRequired[JsonObject]
    messages: NotRequired[list[JsonObject]]


class Input(TypedDict):
    calls: list[Call]


def provider_for(model: str, descriptor: JsonObject | None) -> SimpleProviderConfig:
    slug = model.split("/", 1)[0]
    if descriptor is not None:
        return SimpleProviderConfig(slug=slug, data=descriptor)
    provider = JSONProviderRegistry.get(slug)
    if provider is None:
        raise ValueError(f"{slug} is not a JSON provider")
    return provider


def identity_value(call: Call) -> JsonObject:
    resolved: tuple[str, str, str | None, str | None] = litellm.get_llm_provider(
        model=call["model"], api_base=call.get("api_base"), api_key=call.get("api_key")
    )
    model, provider, key, base = resolved
    return {"model": model, "provider": provider, "base": base, "key": key}


def supported_value(call: Call) -> JsonValue:
    config = FACTORY(provider_for(call["model"], None))()
    return list(config.get_supported_openai_params(call["model"].split("/", 1)[1]))


def params_value(call: Call) -> JsonValue:
    config = FACTORY(provider_for(call["model"], call.get("descriptor")))()
    return config.map_openai_params(
        non_default_params=call["non_default"],
        optional_params=call["optional"],
        model=call["model"].split("/", 1)[1],
        drop_params=False,
    )


def url_value(call: Call) -> JsonValue:
    config = FACTORY(provider_for(call["model"], None))()
    return config.get_complete_url(
        api_base=call.get("api_base"), api_key=None,
        model=call["model"].split("/", 1)[1],
        optional_params={}, litellm_params={},
    )


def messages_value(call: Call) -> JsonValue:
    collapsed = COLLAPSE(call["messages"])
    return MESSAGES.validate_python(collapsed)


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


def run(input_value: Input) -> JsonValue:
    results: list[JsonValue] = []
    for call in input_value["calls"]:
        os.environ.clear()
        environment = call.get("environment")
        if environment is not None:
            os.environ.update(environment)
        results.append(call_value(call))
    return results
