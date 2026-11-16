import asyncio
import datetime as dt
from collections.abc import AsyncIterator
from typing import Final, Literal, TypeAlias, TypedDict, cast

import httpx
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.cisco_ai_defense.cisco_ai_defense import (
    CiscoAIDefenseGuardrail,
)
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.mcp import MCPPostCallResponseObject
from litellm.types.utils import ModelResponseStream
from mcp.types import CallToolResult
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError

GUARDRAIL_NAME: Final = "cisco-ai-defense"
INSPECT_BASE: Final = "http://cisco.inspect"
TIMESTAMP: Final = dt.datetime(2024, 1, 1, tzinfo=dt.UTC)


class Settings(TypedDict, total=False):
    api_key: str
    inspection_type: str
    event_hook: GuardrailEventHooks
    fallback_on_error: str
    on_flagged_action: str


class Answer(TypedDict):
    status: int
    headers: dict[str, str]
    body: JsonValue


class StreamInput(TypedDict):
    operation: Literal["stream"]
    settings: Settings
    request_data: dict[str, JsonValue]
    chunks: list[JsonValue]
    should_run: bool
    answer: Answer


class McpInput(TypedDict):
    operation: Literal["mcp"]
    settings: Settings
    kwargs: dict[str, JsonValue]
    tool_response: list[JsonValue]
    answer: Answer


Input: TypeAlias = StreamInput | McpInput


class ChunkDeltaDump(TypedDict, total=False):
    role: str | None
    content: str | None


class ChunkChoiceDump(TypedDict, total=False):
    index: int | None
    finish_reason: str | None
    delta: ChunkDeltaDump | None


class ChunkDump(TypedDict):
    id: str | None
    object: str | None
    created: int | None
    model: str | None
    choices: list[ChunkChoiceDump]


class ContentItemDump(TypedDict):
    type: str
    text: str


class McpResponseDump(TypedDict):
    mcp_tool_call_response: list[ContentItemDump]


class ToolResultDump(TypedDict):
    content: list[ContentItemDump]


class ChunkShape(BaseModel):
    model_config = ConfigDict(extra="allow")
    choices: list[dict[str, JsonValue]]


class MetadataBucket(BaseModel):
    model_config = ConfigDict(extra="allow")
    applied_guardrails: list[str] | None = None


class StreamRequest(BaseModel):
    model_config = ConfigDict(extra="allow")
    metadata: MetadataBucket | None = None
    litellm_metadata: MetadataBucket | None = None


STREAM_INPUT: Final = TypeAdapter(StreamInput)
MCP_INPUT: Final = TypeAdapter(McpInput)
CHUNK: Final = TypeAdapter(ChunkDump)
CHUNK_SHAPE: Final = TypeAdapter(ChunkShape)
RAW_CHUNK: Final = TypeAdapter(dict[str, JsonValue])
JSON: Final = TypeAdapter(JsonValue)
MCP_RESPONSE: Final = TypeAdapter(McpResponseDump)
TOOL_RESULT: Final = TypeAdapter(ToolResultDump)
STREAM_REQUEST: Final = TypeAdapter(StreamRequest)


class Script:
    def __init__(self, answer: Answer) -> None:
        self.status = answer["status"]
        self.headers = answer["headers"]
        self.body = answer["body"]
        self.calls: list[tuple[str, JsonValue]] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        body: JsonValue = JSON.validate_json(request.content) if request.content else None
        self.calls.append((request.url.path, body))
        return httpx.Response(self.status, json=self.body, headers=self.headers, request=request)


def build_guardrail(settings: Settings, script: Script) -> CiscoAIDefenseGuardrail:
    cisco = CiscoAIDefenseGuardrail(
        guardrail_name=GUARDRAIL_NAME,
        api_key=settings.get("api_key"),
        api_base=INSPECT_BASE,
        inspection_type=settings.get("inspection_type"),
        fallback_on_error=settings.get("fallback_on_error"),
        on_flagged_action=settings.get("on_flagged_action"),
    )
    cisco.event_hook = settings.get("event_hook")
    cisco.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(script))
    return cisco


def parsed_text(text: str) -> JsonValue:
    try:
        parsed = JSON.validate_json(text)
    except ValidationError:
        return text
    return parsed


def sse_value(frame: str) -> JsonValue:
    body = frame.removeprefix("data: ").removesuffix("\n\n")
    try:
        parsed = JSON.validate_json(body)
    except ValidationError:
        return {"kind": "sse", "body": frame}
    return {"kind": "sse", "body": parsed}


def chunk_value(chunk: ChunkDump) -> JsonValue:
    choices: list[JsonValue] = []
    for choice in chunk["choices"]:
        delta = choice.get("delta")
        role: str | None = None
        content: str | None = None
        if delta is not None:
            role = delta.get("role")
            content = delta.get("content")
        choices.append(
            {
                "index": choice.get("index"),
                "finish_reason": choice.get("finish_reason"),
                "role": role,
                "content": content,
            }
        )
    return {
        "kind": "chunk",
        "id": chunk["id"],
        "object": chunk["object"],
        "created": chunk["created"],
        "model": chunk["model"],
        "choices": choices,
    }


def item_value(item: ModelResponseStream | str) -> JsonValue:
    match item:
        case ModelResponseStream():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103-L2106
            return chunk_value(CHUNK.validate_json(item.model_dump_json()))
        case str():
            return sse_value(item)


def recorded(script: Script) -> tuple[JsonValue, JsonValue]:
    path: JsonValue = None
    body: JsonValue = None
    if script.calls:
        first = script.calls[0]
        path = first[0]
        body = first[1]
    return path, body


def applied_guardrails(request_data: dict[str, JsonValue]) -> list[JsonValue]:
    record = STREAM_REQUEST.validate_python(request_data)
    bucket = record.litellm_metadata if record.litellm_metadata is not None else record.metadata
    if bucket is None:
        return []
    names = bucket.applied_guardrails
    if names is None:
        return []
    return [*names]


def stream_items(chunks: list[JsonValue]) -> list[ModelResponseStream | dict[str, JsonValue]]:
    items: list[ModelResponseStream | dict[str, JsonValue]] = []
    for raw in chunks:
        shape = CHUNK_SHAPE.validate_python(raw)
        text_only = any("text" in choice for choice in shape.choices)
        if text_only:
            items.append(RAW_CHUNK.validate_python(raw))
        else:
            items.append(ModelResponseStream.model_validate(raw))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103-L2106
    return items


async def chunk_stream(
    items: list[ModelResponseStream | dict[str, JsonValue]],
) -> AsyncIterator[ModelResponseStream | dict[str, JsonValue]]:
    for item in items:
        yield item


async def stream_async(input_value: StreamInput) -> JsonValue:
    script = Script(input_value["answer"])
    cisco = build_guardrail(input_value["settings"], script)
    request_data = dict(input_value["request_data"])
    responses = cast("AsyncIterator[ModelResponseStream | str]", cisco.async_post_call_streaming_iterator_hook(UserAPIKeyAuth(), chunk_stream(stream_items(input_value["chunks"])), request_data))  # noqa: E501  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/cisco_ai_defense/cisco_ai_defense.py#L419-L537
    yielded: list[JsonValue] = [item_value(item) async for item in responses]
    path, request = recorded(script)
    return {
        "items": yielded,
        "path": path,
        "request": request,
        "applied_guardrails": applied_guardrails(request_data),
    }


async def mcp_async(input_value: McpInput) -> JsonValue:
    script = Script(input_value["answer"])
    cisco = build_guardrail(input_value["settings"], script)
    content = input_value["tool_response"]
    payload: dict[str, JsonValue] = {
        "mcp_tool_call_response": content,
        "hidden_params": {"response_cost": None},
    }
    response = MCPPostCallResponseObject.model_validate(payload)
    original = CallToolResult.model_validate({"content": content})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/cisco_ai_defense/cisco_ai_defense_mcp.py#L97-L173
    kwargs: dict[str, JsonValue | CallToolResult] = dict(input_value["kwargs"])
    if kwargs.get("original_response") is not None:
        kwargs["original_response"] = original
    returned = await cisco.async_post_mcp_tool_call_hook(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/cisco_ai_defense/cisco_ai_defense_mcp.py#L97-L173
        kwargs=kwargs,
        response_obj=response,
        start_time=TIMESTAMP,
        end_time=TIMESTAMP,
    )
    path, request = recorded(script)
    returned_value: JsonValue = None
    if returned is not None:
        returned_dump = MCP_RESPONSE.validate_json(returned.model_dump_json())
        returned_value = [
            {"type": item["type"], "text": parsed_text(item["text"])}
            for item in returned_dump["mcp_tool_call_response"]
        ]
    response_dump = MCP_RESPONSE.validate_json(response.model_dump_json())
    original_dump = TOOL_RESULT.validate_json(original.model_dump_json())
    return {
        "returned": returned_value,
        "response": [
            {"type": item["type"], "text": parsed_text(item["text"])}
            for item in response_dump["mcp_tool_call_response"]
        ],
        "original": [
            {"type": item["type"], "text": parsed_text(item["text"])}
            for item in original_dump["content"]
        ],
        "path": path,
        "request": request,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "stream":
        return asyncio.run(stream_async(STREAM_INPUT.validate_python(input_value)))
    return asyncio.run(mcp_async(MCP_INPUT.validate_python(input_value)))
