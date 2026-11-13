import asyncio
from typing import Literal, Protocol, TypedDict

import httpx
import litellm
from fastapi import HTTPException
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy.guardrails.guardrail_hooks.zscaler_ai_guard.zscaler_ai_guard import (
    ZscalerAIGuard,
)
from litellm.types.llms.custom_http import httpxSpecialProvider
from pydantic import JsonValue, TypeAdapter

CLIENT_CACHE_KEY = "async_httpx_client" + httpxSpecialProvider.LoggingCallback
HEADER_NAMES = (
    "authorization",
    "content-type",
    "user-api-key-alias",
    "user-api-key-team-id",
    "user-api-key-user-id",
)
JSON = TypeAdapter(JsonValue)


class Inputs(TypedDict):
    texts: list[str]


class Guardrail(Protocol):
    async def apply_guardrail(
        self,
        inputs: Inputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
    ) -> dict[str, JsonValue]: ...


class Settings(TypedDict):
    api_base: str | None
    api_key: str | None
    policy_id: int | None
    send_user_api_key_alias: bool | None
    send_user_api_key_user_id: bool | None
    send_user_api_key_team_id: bool | None
    timeout: float | None


class Answer(TypedDict):
    status: int
    body: JsonValue


class Call(TypedDict):
    settings: Settings
    inputs: Inputs
    request_data: dict[str, JsonValue]
    input_type: Literal["request", "response"]
    answer: Answer


class Captured(TypedDict, total=False):
    path: str
    headers: dict[str, JsonValue]
    body: JsonValue


class Handler:
    def __init__(self, answer: Answer, captured: Captured) -> None:
        self.answer = answer
        self.captured = captured

    def __call__(self, request: httpx.Request) -> httpx.Response:
        headers: dict[str, JsonValue] = {
            name: request.headers[name] for name in HEADER_NAMES if name in request.headers
        }
        self.captured["path"] = request.url.path
        self.captured["headers"] = headers
        self.captured["body"] = JSON.validate_json(request.content)
        return httpx.Response(self.answer["status"], json=self.answer["body"], request=request)


def build(settings: Settings) -> Guardrail:
    return ZscalerAIGuard(
        api_key=settings["api_key"],
        api_base=settings["api_base"],
        policy_id=settings["policy_id"],
        send_user_api_key_alias=settings["send_user_api_key_alias"],
        send_user_api_key_user_id=settings["send_user_api_key_user_id"],
        send_user_api_key_team_id=settings["send_user_api_key_team_id"],
        timeout=settings["timeout"],
    )


async def check(
    guardrail: Guardrail, handler: AsyncHTTPHandler, call: Call
) -> dict[str, JsonValue]:
    cache = LLMClientCache()
    litellm.in_memory_llm_clients_cache = cache
    cache.set_cache(CLIENT_CACHE_KEY, handler)
    return await guardrail.apply_guardrail(
        inputs=call["inputs"],
        request_data=call["request_data"],
        input_type=call["input_type"],
    )


def blocked(captured: Captured, error: HTTPException) -> JsonValue:
    detail: JsonValue = error.detail
    return {
        "blocked": True,
        "status": error.status_code,
        "detail": detail,
        "path": captured.get("path"),
        "headers": captured.get("headers"),
        "body": captured.get("body"),
    }


def run(input_value: Call) -> JsonValue:
    captured: Captured = {}
    instance = build(input_value["settings"])
    transport = httpx.MockTransport(Handler(input_value["answer"], captured))
    handler = AsyncHTTPHandler(transport=transport)
    try:
        result = asyncio.run(check(instance, handler, input_value))
    except HTTPException as error:
        return blocked(captured, error)
    hooks = [hook.value for hook in ZscalerAIGuard.get_supported_event_hooks()]
    return {
        "supported_event_hooks": hooks,
        "path": captured.get("path"),
        "headers": captured.get("headers"),
        "body": captured.get("body"),
        "inputs": result,
    }
