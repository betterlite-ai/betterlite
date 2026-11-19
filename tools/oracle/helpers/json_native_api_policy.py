import json
import os
from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.openai_like.dynamic_config import create_responses_config_class
from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig
from litellm.llms.openai_like.messages.transformation import (
    JSONProviderAnthropicMessagesConfig as JSONMessagesConfig,
    OpenAILikeAnthropicMessagesConfig as NativeMessagesConfig,
)
from litellm.llms.openai_like.responses.transformation import OpenAILikeResponsesConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
JSON_OBJECT: Final[TypeAdapter[JsonObject]] = TypeAdapter(dict[str, JsonValue])
EMPTY: Final[JsonObject] = {}
GENERIC_RESPONSES: Final[type[OpenAILikeResponsesConfig]] = OpenAILikeResponsesConfig


class MessagesFactory(Protocol):
    def __call__(self, provider: SimpleProviderConfig) -> NativeMessagesConfig: ...


JSON_MESSAGES: Final[MessagesFactory] = JSONMessagesConfig

CallKind: TypeAlias = Literal[
    "generic_messages_url",
    "json_messages_url",
    "generic_messages_headers",
    "json_messages_headers",
    "generic_messages_transform",
    "json_messages_transform",
    "responses_url",
    "responses_headers",
    "responses_transform",
    "responses_cache",
    "generic_responses_url",
    "generic_responses_headers",
]


class Call(TypedDict):
    kind: CallKind
    model: str
    environment: NotRequired[dict[str, str]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    global_api_key: NotRequired[str | None]
    descriptor: NotRequired[JsonObject | None]
    headers: NotRequired[dict[str, str]]
    optional_params: NotRequired[JsonObject]
    messages: NotRequired[list[JsonObject]]
    input: NotRequired[str]
    params: NotRequired[JsonObject]
    first: NotRequired[JsonObject]
    second: NotRequired[JsonObject]


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


def messages_environment(config: NativeMessagesConfig, call: Call) -> JsonValue:
    merged, _ = config.validate_anthropic_messages_environment(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/messages/transformation.py#L34-L64
        headers=dict(call["headers"]), model=call["model"].split("/", 1)[1],
        messages=call["messages"], optional_params=dict(call["optional_params"]),
        litellm_params=EMPTY, api_key=call.get("api_key"), api_base=call.get("api_base"),
    )
    return JSON_OBJECT.validate_python(merged)


def generic_messages_headers(call: Call) -> JsonValue:
    config = NativeMessagesConfig()
    return messages_environment(config, call)


def json_messages_headers(call: Call) -> JsonValue:
    litellm.api_key = call.get("global_api_key")
    config = JSON_MESSAGES(provider_for(call["model"], call.get("descriptor")))
    return messages_environment(config, call)


def generic_messages_url(call: Call) -> JsonValue:
    config = NativeMessagesConfig()
    return config.get_complete_url(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/messages/transformation.py#L104-L110
        api_base=call.get("api_base"), api_key=None,
        model=call["model"].split("/", 1)[1],
        optional_params=EMPTY, litellm_params=EMPTY,
    )


def json_messages_url(call: Call) -> JsonValue:
    config = JSON_MESSAGES(provider_for(call["model"], call.get("descriptor")))
    return config.get_complete_url(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/messages/transformation.py#L104-L110
        api_base=call.get("api_base"), api_key=None,
        model=call["model"].split("/", 1)[1],
        optional_params=EMPTY, litellm_params=EMPTY,
    )


def generic_messages_transform(call: Call) -> JsonValue:
    body = cast(JsonObject, NativeMessagesConfig().transform_anthropic_messages_request(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/messages/transformation.py#L69-L93
        model=call["model"].split("/", 1)[1], messages=call["messages"],
        anthropic_messages_optional_request_params=dict(call["optional_params"]),
        litellm_params=GenericLiteLLMParams(), headers=dict(EMPTY),
    ))
    return JSON_OBJECT.validate_json(json.dumps(body))


def json_messages_transform(call: Call) -> JsonValue:
    provider = provider_for(call["model"], call.get("descriptor"))
    body = cast(JsonObject, JSON_MESSAGES(provider).transform_anthropic_messages_request(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/messages/transformation.py#L69-L93
        model=call["model"].split("/", 1)[1], messages=call["messages"],
        anthropic_messages_optional_request_params=dict(call["optional_params"]),
        litellm_params=GenericLiteLLMParams(), headers=dict(EMPTY),
    ))
    return JSON_OBJECT.validate_json(json.dumps(body))


def responses_config(call: Call) -> OpenAILikeResponsesConfig:
    provider = provider_for(call["model"], call.get("descriptor"))
    return create_responses_config_class(provider)()


def responses_url(call: Call) -> JsonValue:
    config = responses_config(call)
    return config.get_complete_url(api_base=call.get("api_base"), litellm_params=EMPTY)  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/responses/transformation.py#L42-L51


def responses_headers(call: Call) -> JsonValue:
    merged = cast(JsonObject, responses_config(call).validate_environment(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/responses/transformation.py#L30-L40
        headers=dict(call["headers"]),
        model=call["model"].split("/", 1)[1],
        litellm_params=GenericLiteLLMParams(api_key=call.get("api_key")),
    ))
    return JSON_OBJECT.validate_python(merged)


def responses_transform(call: Call) -> JsonValue:
    body = cast(JsonObject, responses_config(call).transform_responses_api_request(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/dynamic_config.py#L228-L244
        model=call["model"].split("/", 1)[1], input=call["input"],
        response_api_optional_request_params=dict(call["params"]),
        litellm_params=GenericLiteLLMParams(), headers=dict(EMPTY),
    ))
    return JSON_OBJECT.validate_json(json.dumps(body))


def responses_cache(call: Call) -> JsonObject:
    slug = call["model"].split("/", 1)[0]
    first = create_responses_config_class(SimpleProviderConfig(slug=slug, data=call["first"]))
    second = create_responses_config_class(SimpleProviderConfig(slug=slug, data=call["second"]))
    first_url = first().get_complete_url(api_base=None, litellm_params=EMPTY)  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/dynamic_config.py#L211-L226
    second_url = second().get_complete_url(api_base=None, litellm_params=EMPTY)  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/dynamic_config.py#L211-L226
    return {"first": first_url, "second": second_url, "cached": first is second}


def generic_responses_url(call: Call) -> JsonValue:
    config = GENERIC_RESPONSES()
    return config.get_complete_url(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/responses/transformation.py#L42-L51
        api_base=call.get("api_base"), litellm_params=EMPTY
    )


def generic_responses_headers(call: Call) -> JsonValue:
    merged = cast(JsonObject, GENERIC_RESPONSES().validate_environment(  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/responses/transformation.py#L30-L40
        headers=dict(call["headers"]),
        model=call["model"].split("/", 1)[1],
        litellm_params=GenericLiteLLMParams(api_key=call.get("api_key")),
    ))
    return JSON_OBJECT.validate_json(json.dumps(merged))


Handler: TypeAlias = Callable[[Call], JsonValue]
HANDLERS: Final[dict[CallKind, Handler]] = {
    "generic_messages_url": generic_messages_url,
    "json_messages_url": json_messages_url,
    "generic_messages_headers": generic_messages_headers,
    "json_messages_headers": json_messages_headers,
    "generic_messages_transform": generic_messages_transform,
    "json_messages_transform": json_messages_transform,
    "responses_url": responses_url,
    "responses_headers": responses_headers,
    "responses_transform": responses_transform,
    "responses_cache": responses_cache,
    "generic_responses_url": generic_responses_url,
    "generic_responses_headers": generic_responses_headers,
}


def call_value(call: Call) -> JsonValue:
    handler = HANDLERS[call["kind"]]
    return handler(call)


def run(input_value: Input) -> JsonValue:
    results: list[JsonValue] = []
    for call in input_value["calls"]:
        os.environ.clear()
        environment = call.get("environment")
        if environment is not None:
            os.environ.update(environment)
        litellm.api_key = None
        results.append(call_value(call))
    return results
