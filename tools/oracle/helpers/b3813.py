import asyncio
from collections.abc import AsyncIterator
from typing import Final, Literal, NotRequired, TypedDict

from litellm.exceptions import GuardrailRaisedException
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.tool_permission import ToolPermissionGuardrail
from litellm.types.llms.anthropic_messages.anthropic_response import AnthropicMessagesResponse
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter

GUARDRAIL_NAME: Final = "tool-permissions"


class Input(TypedDict):
    rules: list[dict[str, JsonValue]]
    default_action: Literal["allow", "deny"]
    on_disallowed_action: Literal["block", "rewrite"]
    event_hook: Literal["pre_call", "post_call"]
    default_on: bool
    mode: Literal["success", "streaming"]
    format: Literal["openai", "anthropic"]
    data: dict[str, JsonValue]
    chunks: list[dict[str, JsonValue]]
    raw_chunks: list[str]
    violation_message_template: NotRequired[str]
    response: NotRequired[dict[str, JsonValue]]
    max_chunks: NotRequired[int]


class Blocked(TypedDict):
    status: int
    message: str
    blocked_content: bool


class FunctionFields(TypedDict, total=False):
    name: str | None
    arguments: str | None


class ToolCallFields(TypedDict, total=False):
    id: str | None
    type: str | None
    function: FunctionFields | None


class MessageFields(TypedDict, total=False):
    role: str | None
    content: str | None
    tool_calls: list[ToolCallFields] | None
    function_call: FunctionFields | None


class ChoiceFields(TypedDict, total=False):
    index: int | None
    finish_reason: str | None
    message: MessageFields | None
    delta: MessageFields | None


class ChoicesResponse(TypedDict):
    choices: list[ChoiceFields]


class BlockFields(TypedDict, total=False):
    type: str | None
    text: str | None
    id: str | None
    name: str | None


class DeltaFields(TypedDict, total=False):
    type: str | None
    text: str | None
    stop_reason: str | None


class AnthropicContent(TypedDict):
    content: list[BlockFields]


class AnthropicResponseFields(AnthropicContent, total=False):
    stop_reason: str | None


class Envelope(TypedDict, total=False):
    type: str | None


class BlockStartFields(TypedDict, total=False):
    type: str | None
    index: int | None
    content_block: BlockFields | None


class DeltaEventFields(TypedDict, total=False):
    type: str | None
    index: int | None
    delta: DeltaFields | None


class Bucket(TypedDict, total=False):
    applied_guardrails: list[str]


class Buckets(TypedDict, total=False):
    metadata: Bucket | None
    litellm_metadata: Bucket | None


def build_guardrail(input_value: Input) -> ToolPermissionGuardrail:
    return ToolPermissionGuardrail(
        rules=input_value["rules"],
        default_action=input_value["default_action"],
        on_disallowed_action=input_value["on_disallowed_action"],
        guardrail_name=GUARDRAIL_NAME,
        event_hook=input_value["event_hook"],
        default_on=input_value["default_on"],
        violation_message_template=input_value.get("violation_message_template"),
    )


def blocked_of(error: GuardrailRaisedException) -> Blocked:
    return {
        "status": error.status_code,
        "message": error.message,
        "blocked_content": error.blocked_content,
    }


def function_view(function: FunctionFields | None) -> JsonValue:
    if function is None:
        return None
    return TypeAdapter(JsonValue).validate_python(
        {"name": function.get("name"), "arguments": function.get("arguments")}
    )


def tool_call_view(call: ToolCallFields) -> JsonValue:
    return TypeAdapter(JsonValue).validate_python(
        {
            "id": call.get("id"),
            "type": call.get("type"),
            "function": function_view(call.get("function")),
        }
    )


def message_view(message: MessageFields | None) -> JsonValue:
    if message is None:
        return None
    calls = message.get("tool_calls")
    calls_view: list[JsonValue] | None = None
    if calls is not None:
        calls_view = [tool_call_view(call) for call in calls]
    return TypeAdapter(JsonValue).validate_python(
        {
            "role": message.get("role"),
            "content": message.get("content"),
            "tool_calls": calls_view,
            "function_call": function_view(message.get("function_call")),
        }
    )


def choice_view(choice: ChoiceFields) -> JsonValue:
    message = choice.get("message")
    if message is None:
        message = choice.get("delta")
    return TypeAdapter(JsonValue).validate_python(
        {
            "index": choice.get("index"),
            "finish_reason": choice.get("finish_reason"),
            "message": message_view(message),
        }
    )


def choices_view(response: ChoicesResponse) -> JsonValue:
    return TypeAdapter(JsonValue).validate_python(
        [choice_view(choice) for choice in response["choices"]]
    )


def block_view(block: BlockFields) -> JsonValue:
    return TypeAdapter(JsonValue).validate_python(
        {
            "type": block.get("type"),
            "text": block.get("text"),
            "id": block.get("id"),
            "name": block.get("name"),
        }
    )


def anthropic_view(response: AnthropicResponseFields) -> JsonValue:
    return TypeAdapter(JsonValue).validate_python(
        {
            "content": [block_view(block) for block in response["content"]],
            "stop_reason": response.get("stop_reason"),
        }
    )


def applied_guardrails(data: dict[str, JsonValue]) -> list[str] | None:
    buckets = TypeAdapter(Buckets).validate_python(data)
    bucket = buckets.get("metadata")
    if "litellm_metadata" in data:
        bucket = buckets.get("litellm_metadata")
    if bucket is None:
        return None
    return bucket.get("applied_guardrails")


def success(input_value: Input) -> JsonValue:
    guardrail = build_guardrail(input_value)
    payload = input_value.get("response")
    if payload is None:
        raise ValueError("success mode needs a response")
    data = dict(input_value["data"])
    blocked: Blocked | None = None
    if input_value["format"] == "openai":
        response = ModelResponse(**payload)
        try:
            asyncio.run(
                guardrail.async_post_call_success_hook(
                    data=data, user_api_key_dict=UserAPIKeyAuth(), response=response
                )
            )
        except GuardrailRaisedException as error:
            blocked = blocked_of(error)
        dump = TypeAdapter(ChoicesResponse).validate_json(response.model_dump_json())
        response_view = choices_view(dump)
    else:
        anthropic_response = TypeAdapter(AnthropicMessagesResponse).validate_python(payload)
        try:
            asyncio.run(
                guardrail.async_post_call_success_hook(
                    data=data, user_api_key_dict=UserAPIKeyAuth(), response=anthropic_response
                )
            )
        except GuardrailRaisedException as error:
            blocked = blocked_of(error)
        response_view = anthropic_view(
            TypeAdapter(AnthropicResponseFields).validate_python(anthropic_response)
        )
    return TypeAdapter(JsonValue).validate_python(
        {
            "blocked": blocked,
            "response": response_view,
            "applied_guardrails": applied_guardrails(data),
        }
    )


def event_view(event: JsonValue) -> JsonValue:
    envelope = TypeAdapter(Envelope).validate_python(event)
    kind = envelope.get("type")
    if kind == "content_block_start":
        block_start = TypeAdapter(BlockStartFields).validate_python(event)
        block = block_start.get("content_block")
        return TypeAdapter(JsonValue).validate_python(
            {
                "type": kind,
                "index": block_start.get("index"),
                "content_block": None if block is None else block_view(block),
            }
        )
    if kind == "content_block_delta":
        delta_event = TypeAdapter(DeltaEventFields).validate_python(event)
        delta = delta_event.get("delta")
        return TypeAdapter(JsonValue).validate_python(
            {
                "type": kind,
                "index": delta_event.get("index"),
                "delta": None
                if delta is None
                else {"type": delta.get("type"), "text": delta.get("text")},
            }
        )
    if kind == "content_block_stop":
        stop_event = TypeAdapter(DeltaEventFields).validate_python(event)
        return TypeAdapter(JsonValue).validate_python(
            {"type": kind, "index": stop_event.get("index")}
        )
    if kind == "message_delta":
        message_delta = TypeAdapter(DeltaEventFields).validate_python(event)
        delta = message_delta.get("delta")
        return TypeAdapter(JsonValue).validate_python(
            {
                "type": kind,
                "delta": None
                if delta is None
                else {"stop_reason": delta.get("stop_reason")},
            }
        )
    return TypeAdapter(JsonValue).validate_python({"type": kind})


def data_payload(frame: str) -> JsonValue | None:
    for line in frame.splitlines():
        stripped = line.strip()
        if not stripped.startswith("data:"):
            continue
        payload = stripped.removeprefix("data:").strip()
        if payload in ("", "[DONE]"):
            return None
        return TypeAdapter(JsonValue).validate_json(payload)
    return None


def raw_chunk_view(text: str) -> JsonValue:
    events: list[JsonValue] = []
    for frame in text.split("\n\n"):
        payload = data_payload(frame)
        if payload is not None:
            events.append(event_view(payload))
    return TypeAdapter(JsonValue).validate_python({"events": events})


async def openai_source(chunks: list[dict[str, JsonValue]]) -> AsyncIterator[ModelResponseStream]:
    for chunk in chunks:
        yield ModelResponseStream(**chunk)


async def raw_source(chunks: list[str]) -> AsyncIterator[str]:
    for chunk in chunks:
        yield chunk


Streamed = ModelResponseStream | bytes | str


async def collect_stream(stream: AsyncIterator[Streamed]) -> list[Streamed]:
    collected: list[Streamed] = []
    async for item in stream:
        collected.append(item)
    return collected


def streamed_view(item: Streamed) -> JsonValue:
    match item:
        case bytes():
            return raw_chunk_view(item.decode("utf-8"))
        case str():
            return raw_chunk_view(item)
        case ModelResponseStream():
            chunk = TypeAdapter(ChoicesResponse).validate_json(item.model_dump_json())
            return TypeAdapter(JsonValue).validate_python({"choices": choices_view(chunk)})


def streaming(input_value: Input) -> JsonValue:
    guardrail = build_guardrail(input_value)
    data = dict(input_value["data"])
    blocked: Blocked | None = None
    if input_value["raw_chunks"]:
        source: AsyncIterator[ModelResponseStream] | AsyncIterator[str] = raw_source(
            input_value["raw_chunks"]
        )
    else:
        source = openai_source(input_value["chunks"])
    collected: list[Streamed] = []
    try:
        hook = guardrail.async_post_call_streaming_iterator_hook(  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/tool_permission.py#L892-L897
            UserAPIKeyAuth(), source, data
        )
        collected = asyncio.run(collect_stream(hook))
    except GuardrailRaisedException as error:
        blocked = blocked_of(error)
    return TypeAdapter(JsonValue).validate_python(
        {
            "chunks": [streamed_view(item) for item in collected],
            "blocked": blocked,
        }
    )


def run(input_value: Input) -> JsonValue:
    if input_value["mode"] == "success":
        return success(input_value)
    return streaming(input_value)
