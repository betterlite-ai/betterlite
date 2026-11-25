import asyncio
import json
from typing import TypedDict

from litellm.proxy.common_utils.openai_endpoint_utils import (
    get_custom_llm_provider_from_request_body,
    get_custom_llm_provider_from_request_headers,
    get_custom_llm_provider_from_request_query,
)
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Message, Scope


class Input(TypedDict):
    headers: list[tuple[str, str]]
    query: str | None
    body: dict[str, JsonValue]


class Answer(TypedDict):
    headers: str | None
    query: str | None
    body: JsonValue


class BodyReceiver:
    def __init__(self, body: bytes) -> None:
        self.body = body
        self.sent = False

    async def __call__(self) -> Message:
        if self.sent:
            return {"type": "http.disconnect"}
        self.sent = True
        return {"type": "http.request", "body": self.body, "more_body": False}


def header_pairs(headers: list[tuple[str, str]]) -> list[tuple[bytes, bytes]]:
    # an ASGI server lowercases header names before Starlette compares them
    return [
        (name.lower().encode("latin-1"), value.encode("latin-1"))
        for name, value in headers
    ]


def query_bytes(query: str | None) -> bytes:
    if query is None:
        return b""
    return query.encode("latin-1")


def build_request(input_value: Input) -> Request:
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": "/v1/files",
        "query_string": query_bytes(input_value["query"]),
        "headers": header_pairs(input_value["headers"]),
    }
    return Request(scope, BodyReceiver(json.dumps(input_value["body"]).encode()))


async def answer(input_value: Input) -> Answer:
    request = build_request(input_value)
    return {
        "headers": get_custom_llm_provider_from_request_headers(request=request),
        "query": get_custom_llm_provider_from_request_query(request=request),
        "body": await get_custom_llm_provider_from_request_body(request=request),
    }


def run(input_value: Input) -> Answer:
    parsed = TypeAdapter(Input).validate_python(input_value)
    return asyncio.run(answer(parsed))
