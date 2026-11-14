import os

import httpx
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue

from .configs import CHAT, EMBEDDING, NoMessages, logging_object
from .views import (
    REPLY,
    USAGE,
    HeadersCall,
    MapCall,
    RequestCall,
    ResponseCall,
    SupportedCall,
    UrlCall,
)


def replace_environment(values: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(values)


def chat_supported_value(call: SupportedCall) -> JsonValue:
    params: list[JsonValue] = [*CHAT.get_supported_openai_params(call["model"])]
    return params


def chat_map_value(call: MapCall) -> JsonValue:
    optional: dict[str, JsonValue] = dict(call["preset"])
    mapped: dict[str, JsonValue] = CHAT.map_openai_params(
        dict(call["params"]), optional, call["model"], drop_params=False
    )
    return mapped


def embedding_headers_value(call: HeadersCall) -> JsonValue:
    replace_environment(call["env"])
    headers: dict[str, JsonValue] = EMBEDDING.validate_environment(
        dict(call["headers"]),
        "m",
        NoMessages(),
        {},
        {},
        call["api_key"],
    )
    return headers


def embedding_url_value(call: UrlCall) -> JsonValue:
    replace_environment(call["env"])
    return EMBEDDING.get_complete_url(call["api_base"], None, "m", {}, {})


def embedding_request_value(call: RequestCall) -> JsonValue:
    headers: dict[str, str] = {}
    body: dict[str, JsonValue] = EMBEDDING.transform_embedding_request(
        call["model"], call["input"], call["optional_params"], headers
    )
    return body


def reply_value(response: EmbeddingResponse) -> JsonValue:
    parsed = REPLY.validate_json(response.model_dump_json())
    usage = USAGE.validate_python(parsed.usage)
    counts: dict[str, JsonValue] = {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
    if usage is not None:
        counts["prompt_tokens"] = usage.prompt_tokens
        counts["completion_tokens"] = usage.completion_tokens
        counts["total_tokens"] = usage.total_tokens
    value: dict[str, JsonValue] = {
        "model": parsed.model,
        "object": parsed.object,
        "data": parsed.data,
        "usage": counts,
    }
    return value


def embedding_response_value(call: ResponseCall) -> JsonValue:
    raw = httpx.Response(
        200,
        content=call["body"].encode(),
        request=httpx.Request("POST", "https://host"),
    )
    response = EMBEDDING.transform_embedding_response(
        "m", raw, EmbeddingResponse(), logging_object(), None, {}, {}, {}
    )
    return reply_value(response)
