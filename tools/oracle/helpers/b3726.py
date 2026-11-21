import asyncio
import time
from collections.abc import AsyncGenerator, AsyncIterator, Awaitable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.caching.dual_cache import DualCache
from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.bedrock_guardrails import BedrockGuardrail
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import CallTypesLiteral, ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
DICT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
LIST: Final[TypeAdapter[list[JsonValue]]] = TypeAdapter(list[JsonValue])
STRING: Final[TypeAdapter[str]] = TypeAdapter(str)
EVENT: Final[TypeAdapter[GuardrailEventHooks | None]] = TypeAdapter(GuardrailEventHooks | None)
CHUNK: Final[TypeAdapter[ModelResponseStream]] = TypeAdapter(ModelResponseStream)
MODEL: Final[TypeAdapter[ModelResponse]] = TypeAdapter(ModelResponse)
ITEM: Final[TypeAdapter[bytes | ModelResponseStream]] = TypeAdapter(bytes | ModelResponseStream)

Payload: TypeAlias = dict[str, JsonValue]
StreamItem: TypeAlias = ModelResponseStream | bytes


class GuardrailSetup(TypedDict):
    name: str
    identifier: str
    version: str
    disable_exception_on_block: bool | None
    latest_role_message_only: bool | None
    event_hook: str | None
    buffer_until_moderated: bool | None
    streaming_end_of_stream_only: bool | None
    buffer_release_on_scan: bool | None


class Reply(TypedDict):
    status: int
    body: JsonValue


class HookCase(TypedDict):
    hook: Literal["pre_call", "moderation", "post_call", "streaming"]
    call_type: CallTypesLiteral
    guardrail: GuardrailSetup
    data: Payload
    response: JsonValue
    chunks: list[Payload]
    sse_chunks: list[str]
    request_route: str | None
    keepalive_interval_seconds: float | None
    replies: list[Reply]


class Hooks(Protocol):
    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: Payload,
        call_type: CallTypesLiteral,
    ) -> object: ...

    async def async_moderation_hook(
        self, data: Payload, user_api_key_dict: UserAPIKeyAuth, call_type: CallTypesLiteral
    ) -> object: ...

    async def async_post_call_success_hook(
        self, data: Payload, user_api_key_dict: UserAPIKeyAuth, response: ModelResponse
    ) -> object: ...

    def async_post_call_streaming_iterator_hook(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        response: AsyncIterator[StreamItem],
        request_data: Payload,
    ) -> AsyncGenerator[StreamItem, None]: ...


class Scripted:
    def __init__(self, replies: list[Reply], seen: list[JsonValue]) -> None:
        self.replies = replies
        self.seen = seen
        self.used = 0

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(JSON.validate_json(request.content))
        # The scan runs before the keepalive decision, and real time is what that decision reads.
        time.sleep(0.05)
        if self.used >= len(self.replies):
            raise AssertionError("ApplyGuardrail sent more requests than the case scripts")
        reply = self.replies[self.used]
        self.used += 1
        return httpx.Response(reply["status"], json=reply["body"])


def rejected(error: HTTPException) -> JsonValue:
    detail = JSON.validate_python(error.detail)
    return {
        "kind": "rejected",
        "status_code": error.status_code,
        "detail": detail,
    }


async def attempt(call: Awaitable[object]) -> JsonValue:
    try:
        await call
    except HTTPException as error:
        return rejected(error)
    return {"kind": "allowed"}


# The caller-visible chunk fields: the model, the assistant text, the usage and the finish reason.
def chat_json(chunk: Payload) -> JsonValue:
    text = ""
    finish: JsonValue = None
    choices = chunk.get("choices")
    if choices is not None:
        for choice in LIST.validate_python(choices):
            record = DICT.validate_python(choice)
            delta = record.get("delta")
            part = None if delta is None else DICT.validate_python(delta).get("content")
            if part is not None:
                text += STRING.validate_python(part)
            reason = record.get("finish_reason")
            if reason is not None:
                finish = reason
    usage = chunk.get("usage")
    rendered: JsonValue = None
    if usage is not None:
        tokens = DICT.validate_python(usage)
        rendered = {
            "prompt_tokens": tokens.get("prompt_tokens"),
            "completion_tokens": tokens.get("completion_tokens"),
            "total_tokens": tokens.get("total_tokens"),
        }
    return {
        "model": chunk.get("model"),
        "text": text,
        "usage": rendered,
        "finish_reason": finish,
    }


def frames_json(text: str) -> JsonValue:
    parsed: list[JsonValue] = []
    for block in text.split("\n\n"):
        lines = [line for line in block.split("\n") if line.startswith("data:")]
        if not lines:
            continue
        try:
            frame = JSON.validate_json(lines[0][len("data:") :].strip())
        except ValueError:
            return {"raw": text}
        parsed.append(frame)
    if not parsed:
        return {"raw": text}
    return JSON.validate_python({"frames": parsed})


def item_json(item: StreamItem) -> JsonValue:
    payload = JSON.validate_json(ITEM.dump_json(item))
    match payload:
        case str():
            return frames_json(payload)
        case dict():
            return chat_json(DICT.validate_python(payload))
        case None | bool() | int() | float() | list():
            return None


async def stream_items(case: HookCase) -> AsyncGenerator[StreamItem, None]:
    if case["sse_chunks"]:
        for frame in case["sse_chunks"]:
            yield frame.encode("utf-8")
    else:
        for chunk in case["chunks"]:
            yield CHUNK.validate_python(chunk)


async def hook_outcome(
    hooks: Hooks, case: HookCase, data: Payload, user: UserAPIKeyAuth
) -> tuple[JsonValue, list[JsonValue]]:
    outcome: JsonValue = None
    chunks: list[JsonValue] = []
    match case["hook"]:
        case "pre_call":
            outcome = await attempt(
                hooks.async_pre_call_hook(user, DualCache(), data, case["call_type"])
            )
        case "moderation":
            outcome = await attempt(hooks.async_moderation_hook(data, user, case["call_type"]))
        case "post_call":
            response = MODEL.validate_python(case["response"])
            outcome = await attempt(hooks.async_post_call_success_hook(data, user, response))
        case "streaming":
            stream = hooks.async_post_call_streaming_iterator_hook(user, stream_items(case), data)
            try:
                chunks.extend([item_json(item) async for item in stream])
            except HTTPException as error:
                return rejected(error), []
            outcome = {"kind": "allowed"}
    return outcome, chunks


def run(input_value: HookCase) -> JsonValue:
    setup = input_value["guardrail"]
    guardrail = BedrockGuardrail(
        guardrail_name=setup["name"],
        guardrailIdentifier=setup["identifier"],
        guardrailVersion=setup["version"],
        disable_exception_on_block=setup["disable_exception_on_block"],
        experimental_use_latest_role_message_only=setup["latest_role_message_only"],
        event_hook=EVENT.validate_python(setup["event_hook"]),
        streaming_buffer_until_moderated=setup["buffer_until_moderated"],
        streaming_end_of_stream_only=setup["streaming_end_of_stream_only"],
        streaming_buffer_release_on_scan=setup["buffer_release_on_scan"],
        aws_access_key_id="AKIAIOSFODNN7EXAMPLE",
        aws_secret_access_key="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        aws_region_name="us-west-2",
    )
    hooks: Hooks = guardrail
    seen: list[JsonValue] = []
    guardrail.async_handler.client = httpx.AsyncClient(
        transport=httpx.MockTransport(Scripted(input_value["replies"], seen))
    )
    data = dict(input_value["data"])
    user = UserAPIKeyAuth(request_route=input_value["request_route"])
    premium = proxy_server.premium_user
    proxy_server.premium_user = True
    interval = litellm.anthropic_sse_ping_interval_seconds
    configured = input_value["keepalive_interval_seconds"]
    if configured is not None:
        litellm.anthropic_sse_ping_interval_seconds = configured
    try:
        outcome, chunks = asyncio.run(hook_outcome(hooks, input_value, data, user))
    finally:
        proxy_server.premium_user = premium
        litellm.anthropic_sse_ping_interval_seconds = interval
    return {"outcome": outcome, "chunks": chunks, "scans": len(seen)}
