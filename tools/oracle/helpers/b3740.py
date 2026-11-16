import asyncio
import threading
from collections.abc import AsyncIterator
from typing import Literal, TypedDict

import httpx
from fastapi import HTTPException
from litellm.caching.dual_cache import DualCache
from litellm.exceptions import APIError
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.microsoft_purview.purview_dlp import (
    MicrosoftPurviewDLPGuardrail,
)
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.utils import (
    CallTypesLiteral,
    ModelResponse,
    ModelResponseStream,
    StreamingChoices,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from pydantic.alias_generators import to_camel

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
RESPONSE: TypeAdapter[ModelResponse] = TypeAdapter(ModelResponse)


class GuardrailConfig(TypedDict):
    guardrail_name: str
    tenant_id: str
    client_id: str
    client_secret: str
    purview_app_name: str
    user_id_field: str
    timeout_seconds: int


class Reply(TypedDict):
    status: int
    body: JsonValue


class Identity(TypedDict):
    user_id: str | None
    end_user_id: str | None


class Input(TypedDict):
    function: Literal[
        "pre_call",
        "post_success",
        "streaming",
        "logging_sync",
        "logging_deferred",
        "logging_async",
    ]
    guardrail: GuardrailConfig
    replies: list[Reply]
    identity: Identity
    call_type: CallTypesLiteral
    data: dict[str, JsonValue]
    result: JsonValue
    chunks: list[JsonValue]


class TextContent(BaseModel):
    data: str


class ContentEntry(BaseModel):
    content: TextContent


class ActivityMetadata(BaseModel):
    activity: str


class ContentToProcess(BaseModel):
    model_config = ConfigDict(alias_generator=to_camel, populate_by_name=True)

    content_entries: list[ContentEntry]
    activity_metadata: ActivityMetadata


class ProcessBody(BaseModel):
    model_config = ConfigDict(alias_generator=to_camel, populate_by_name=True)

    content_to_process: ContentToProcess


PROCESS: TypeAdapter[ProcessBody] = TypeAdapter(ProcessBody)


class ScriptedEvent(ModelResponseStream):
    id: str = ""
    created: int = 0
    object: str = ""
    type: str = ""
    response: ResponsesAPIResponse | None = None
    choices: list[StreamingChoices] = []


EVENT: TypeAdapter[ScriptedEvent] = TypeAdapter(ScriptedEvent)


class ScriptedReplies:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = replies
        self.scans: list[dict[str, str]] = []
        self.used = 0

    def __call__(self, request: httpx.Request) -> httpx.Response:
        path = request.url.path
        if path.endswith("/processContent"):
            process = PROCESS.validate_json(request.content).content_to_process
            self.scans.append(
                {
                    "activity": process.activity_metadata.activity,
                    "data": process.content_entries[0].content.data,
                }
            )
            if self.used >= len(self.replies):
                return httpx.Response(500, content=b"{}")
            reply = self.replies[self.used]
            self.used += 1
            return httpx.Response(reply["status"], json=reply["body"])
        if path.endswith("/protectionScopes/compute"):
            return httpx.Response(200, content=b'{"value":[]}')
        return httpx.Response(200, content=b'{"access_token":"tok","expires_in":3600}')


async def chunk_stream(chunks: list[ModelResponseStream]) -> AsyncIterator[ModelResponseStream]:
    for chunk in chunks:
        yield chunk


def pre_call(hooks: MicrosoftPurviewDLPGuardrail, case: Input) -> int | None:
    credentials = UserAPIKeyAuth(user_id=case["identity"]["user_id"])
    try:
        asyncio.run(
            hooks.async_pre_call_hook(credentials, DualCache(), case["data"], case["call_type"])
        )
    except HTTPException as error:
        return error.status_code
    return None


def post_success(hooks: MicrosoftPurviewDLPGuardrail, case: Input) -> int | None:
    response = RESPONSE.validate_python(case["result"])
    credentials = UserAPIKeyAuth(user_id=case["identity"]["user_id"])
    try:
        asyncio.run(hooks.async_post_call_success_hook(case["data"], credentials, response))
    except HTTPException as error:
        return error.status_code
    return None


async def streaming(hooks: MicrosoftPurviewDLPGuardrail, case: Input) -> int | None:
    chunks: list[ModelResponseStream] = [EVENT.validate_python(value) for value in case["chunks"]]
    credentials = UserAPIKeyAuth(user_id=case["identity"]["user_id"])
    try:
        stream = hooks.async_post_call_streaming_iterator_hook(
            credentials, chunk_stream(chunks), case["data"]
        )
        async for _chunk in stream:
            pass
    except HTTPException as error:
        return error.status_code
    except APIError as error:
        return error.status_code
    return None


def logging_sync(hooks: MicrosoftPurviewDLPGuardrail, case: Input) -> int | None:
    before = set(threading.enumerate())
    hooks.logging_hook(case["data"], RESPONSE.validate_python(case["result"]), case["call_type"])
    for thread in set(threading.enumerate()) - before:
        thread.join()
    return None


async def logging_deferred(hooks: MicrosoftPurviewDLPGuardrail, case: Input) -> int | None:
    hooks.logging_hook(case["data"], RESPONSE.validate_python(case["result"]), case["call_type"])
    return None


async def logging_async(hooks: MicrosoftPurviewDLPGuardrail, case: Input) -> int | None:
    await hooks.async_logging_hook(
        case["data"], RESPONSE.validate_python(case["result"]), case["call_type"]
    )
    return None


def run(input_value: Input) -> JsonValue:
    handler = ScriptedReplies(input_value["replies"])
    config = input_value["guardrail"]
    hooks = MicrosoftPurviewDLPGuardrail(
        guardrail_name=config["guardrail_name"],
        tenant_id=config["tenant_id"],
        client_id=config["client_id"],
        client_secret=config["client_secret"],
        purview_app_name=config["purview_app_name"],
        user_id_field=config["user_id_field"],
        timeout=config["timeout_seconds"],
    )
    hooks.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handler))
    function = input_value["function"]
    if function == "pre_call":
        status = pre_call(hooks, input_value)
    elif function == "post_success":
        status = post_success(hooks, input_value)
    elif function == "streaming":
        status = asyncio.run(streaming(hooks, input_value))
    elif function == "logging_sync":
        status = logging_sync(hooks, input_value)
    elif function == "logging_deferred":
        status = asyncio.run(logging_deferred(hooks, input_value))
    elif function == "logging_async":
        status = asyncio.run(logging_async(hooks, input_value))
    else:
        raise ValueError(f"unsupported function {function}")
    return {"scans": JSON.validate_python(handler.scans), "status": status}
