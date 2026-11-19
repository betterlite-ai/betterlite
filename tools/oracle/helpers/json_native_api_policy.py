import json
import os
from typing import Any, Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.llms.openai_like.dynamic_config import create_responses_config_class
from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig
from litellm.llms.openai_like.messages.transformation import (
    JSONProviderAnthropicMessagesConfig,
    OpenAILikeAnthropicMessagesConfig,
)
from litellm.llms.openai_like.responses.transformation import OpenAILikeResponsesConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
JSON_OBJECT: Final[TypeAdapter[JsonObject]] = TypeAdapter(dict[str, JsonValue])


class CallBase(TypedDict):
    model: str
    environment: NotRequired[dict[str, str]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    global_api_key: NotRequired[str | None]
    descriptor: NotRequired[JsonObject | None]


class GenericMessagesUrlCall(CallBase):
    kind: Literal["generic_messages_url"]


class JsonMessagesUrlCall(CallBase):
    kind: Literal["json_messages_url"]


class GenericMessagesHeadersCall(CallBase):
    kind: Literal["generic_messages_headers"]
    headers: dict[str, str]
    optional_params: JsonObject
    # pydantic cannot validate LiteLLM's message union without materializing lazy iterables,
    # so the case's plain message dicts reach the config through Any
    messages: list[Any]


class JsonMessagesHeadersCall(CallBase):
    kind: Literal["json_messages_headers"]
    headers: dict[str, str]
    optional_params: JsonObject
    messages: list[Any]


class GenericMessagesTransformCall(CallBase):
    kind: Literal["generic_messages_transform"]
    optional_params: JsonObject
    messages: list[Any]


class JsonMessagesTransformCall(CallBase):
    kind: Literal["json_messages_transform"]
    optional_params: JsonObject
    messages: list[Any]


class ResponsesUrlCall(CallBase):
    kind: Literal["responses_url"]


class ResponsesHeadersCall(CallBase):
    kind: Literal["responses_headers"]
    headers: dict[str, str]


class ResponsesTransformCall(CallBase):
    kind: Literal["responses_transform"]
    input: str
    params: JsonObject


class ResponsesCacheCall(CallBase):
    kind: Literal["responses_cache"]
    first: JsonObject
    second: JsonObject


class GenericResponsesUrlCall(CallBase):
    kind: Literal["generic_responses_url"]


class GenericResponsesHeadersCall(CallBase):
    kind: Literal["generic_responses_headers"]
    headers: dict[str, str]


Call: TypeAlias = (
    GenericMessagesUrlCall
    | JsonMessagesUrlCall
    | GenericMessagesHeadersCall
    | JsonMessagesHeadersCall
    | GenericMessagesTransformCall
    | JsonMessagesTransformCall
    | ResponsesUrlCall
    | ResponsesHeadersCall
    | ResponsesTransformCall
    | ResponsesCacheCall
    | GenericResponsesUrlCall
    | GenericResponsesHeadersCall
)


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


def messages_config(model: str, descriptor: JsonObject | None) -> JSONProviderAnthropicMessagesConfig:
    return JSONProviderAnthropicMessagesConfig(provider_for(model, descriptor))


def responses_config(model: str, descriptor: JsonObject | None) -> OpenAILikeResponsesConfig:
    return create_responses_config_class(provider_for(model, descriptor))()


def generic_messages_url(call: GenericMessagesUrlCall) -> JsonValue:
    return OpenAILikeAnthropicMessagesConfig().get_complete_url(
        api_base=call.get("api_base"),
        api_key=None,
        model=stripped_model(call["model"]),
        optional_params={},
        litellm_params={},
    )


def json_messages_url(call: JsonMessagesUrlCall) -> JsonValue:
    return messages_config(call["model"], call.get("descriptor")).get_complete_url(
        api_base=call.get("api_base"),
        api_key=None,
        model=stripped_model(call["model"]),
        optional_params={},
        litellm_params={},
    )


def generic_messages_headers(call: GenericMessagesHeadersCall) -> JsonValue:
    merged, _ = OpenAILikeAnthropicMessagesConfig().validate_anthropic_messages_environment(
        headers=dict(call["headers"]),
        model=stripped_model(call["model"]),
        messages=call["messages"],
        optional_params=dict(call["optional_params"]),
        litellm_params={},
        api_key=call.get("api_key"),
        api_base=call.get("api_base"),
    )
    return JSON_OBJECT.validate_python(merged)


def json_messages_headers(call: JsonMessagesHeadersCall) -> JsonValue:
    config = messages_config(call["model"], call.get("descriptor"))
    litellm.api_key = call.get("global_api_key")
    merged, _ = config.validate_anthropic_messages_environment(
        headers=dict(call["headers"]),
        model=stripped_model(call["model"]),
        messages=call["messages"],
        optional_params=dict(call["optional_params"]),
        litellm_params={},
        api_key=call.get("api_key"),
        api_base=call.get("api_base"),
    )
    return JSON_OBJECT.validate_python(merged)


def generic_messages_transform(call: GenericMessagesTransformCall) -> JsonValue:
    body = OpenAILikeAnthropicMessagesConfig().transform_anthropic_messages_request(
        model=stripped_model(call["model"]),
        messages=call["messages"],
        anthropic_messages_optional_request_params=dict(call["optional_params"]),
        litellm_params=GenericLiteLLMParams(),
        headers={},
    )
    return JSON_OBJECT.validate_json(json.dumps(body))


def json_messages_transform(call: JsonMessagesTransformCall) -> JsonValue:
    config = messages_config(call["model"], call.get("descriptor"))
    body = config.transform_anthropic_messages_request(
        model=stripped_model(call["model"]),
        messages=call["messages"],
        anthropic_messages_optional_request_params=dict(call["optional_params"]),
        litellm_params=GenericLiteLLMParams(),
        headers={},
    )
    return JSON_OBJECT.validate_json(json.dumps(body))


def responses_url(call: ResponsesUrlCall) -> JsonValue:
    config = responses_config(call["model"], call.get("descriptor"))
    return config.get_complete_url(api_base=call.get("api_base"), litellm_params={})


def responses_headers(call: ResponsesHeadersCall) -> JsonValue:
    config = responses_config(call["model"], call.get("descriptor"))
    merged = config.validate_environment(
        headers=dict(call["headers"]),
        model=stripped_model(call["model"]),
        litellm_params=GenericLiteLLMParams(api_key=call.get("api_key")),
    )
    return JSON_OBJECT.validate_json(json.dumps(merged))


def responses_transform(call: ResponsesTransformCall) -> JsonValue:
    config = responses_config(call["model"], call.get("descriptor"))
    body = config.transform_responses_api_request(
        model=stripped_model(call["model"]),
        input=call["input"],
        response_api_optional_request_params=dict(call["params"]),
        litellm_params=GenericLiteLLMParams(),
        headers={},
    )
    return JSON_OBJECT.validate_json(json.dumps(body))


def responses_cache(call: ResponsesCacheCall) -> JsonValue:
    slug = call["model"].split("/", 1)[0]
    first_class = create_responses_config_class(SimpleProviderConfig(slug=slug, data=call["first"]))
    second_class = create_responses_config_class(SimpleProviderConfig(slug=slug, data=call["second"]))
    first_url = first_class().get_complete_url(api_base=None, litellm_params={})
    second_url = second_class().get_complete_url(api_base=None, litellm_params={})
    result: dict[str, JsonValue] = {
        "first": first_url,
        "second": second_url,
        "cached": first_class is second_class,
    }
    return result


def generic_responses_url(call: GenericResponsesUrlCall) -> JsonValue:
    return OpenAILikeResponsesConfig().get_complete_url(api_base=call.get("api_base"), litellm_params={})


def generic_responses_headers(call: GenericResponsesHeadersCall) -> JsonValue:
    merged = OpenAILikeResponsesConfig().validate_environment(
        headers=dict(call["headers"]),
        model=stripped_model(call["model"]),
        litellm_params=GenericLiteLLMParams(api_key=call.get("api_key")),
    )
    return JSON_OBJECT.validate_json(json.dumps(merged))


def call_value(call: Call) -> JsonValue:
    if call["kind"] == "generic_messages_url":
        return generic_messages_url(call)
    if call["kind"] == "json_messages_url":
        return json_messages_url(call)
    if call["kind"] == "generic_messages_headers":
        return generic_messages_headers(call)
    if call["kind"] == "json_messages_headers":
        return json_messages_headers(call)
    if call["kind"] == "generic_messages_transform":
        return generic_messages_transform(call)
    if call["kind"] == "json_messages_transform":
        return json_messages_transform(call)
    if call["kind"] == "responses_url":
        return responses_url(call)
    if call["kind"] == "responses_headers":
        return responses_headers(call)
    if call["kind"] == "responses_transform":
        return responses_transform(call)
    if call["kind"] == "responses_cache":
        return responses_cache(call)
    if call["kind"] == "generic_responses_url":
        return generic_responses_url(call)
    return generic_responses_headers(call)


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    results: list[JsonValue] = []
    for call in parsed["calls"]:
        os.environ.clear()
        environment = call.get("environment")
        if environment is not None:
            os.environ.update(environment)
        litellm.api_key = None
        results.append(call_value(call))
    return results
