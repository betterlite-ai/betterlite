import asyncio
import contextlib
import io
from collections.abc import AsyncIterator
from typing import Final, Literal, NotRequired, TypedDict

from litellm.exceptions import GuardrailRaisedException
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.tool_permission import ToolPermissionGuardrail
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


class Bag(TypedDict, total=False):
    type: str | None
    text: str | None
    id: str | None
    name: str | None
    index: int | None
    role: str | None
    content: str | None
    arguments: str | None
    finish_reason: str | None
    stop_reason: str | None
    tool_calls: "list[Bag] | None"
    function_call: "Bag | None"
    function: "Bag | None"
    message: "Bag | None"
    delta: "Bag | None"
    metadata: "Bag | None"
    litellm_metadata: "Bag | None"
    applied_guardrails: list[str] | None


class Choices(TypedDict):
    choices: list[Bag]


class AnthropicResponse(TypedDict, total=False):
    content: list[Bag]
    stop_reason: str | None


FIELDS: TypeAdapter[Bag] = TypeAdapter(Bag)
ANTHROPIC: TypeAdapter[AnthropicResponse] = TypeAdapter(AnthropicResponse)


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


def blocked_of(error: GuardrailRaisedException) -> dict[str, JsonValue]:
    view: dict[str, JsonValue] = {
        "status": error.status_code,
        "message": error.message,
        "blocked_content": error.blocked_content,
    }
    return view


def function_view(function: Bag | None) -> JsonValue:
    if function is None:
        return None
    view: dict[str, JsonValue] = {"name": function.get("name")}
    view["arguments"] = function.get("arguments")
    return view


def tool_call_view(call: Bag) -> JsonValue:
    view: dict[str, JsonValue] = {"id": call.get("id"), "type": call.get("type")}
    view["function"] = function_view(call.get("function"))
    return view


def message_view(message: Bag | None) -> JsonValue:
    if message is None:
        return None
    calls = message.get("tool_calls")
    calls_view: list[JsonValue] | None = None
    if calls is not None:
        calls_view = [tool_call_view(call) for call in calls]
    view: dict[str, JsonValue] = {
        "role": message.get("role"),
        "content": message.get("content"),
        "tool_calls": calls_view,
        "function_call": function_view(message.get("function_call")),
    }
    return view


def choice_view(choice: Bag) -> JsonValue:
    message = choice.get("message")
    if message is None:
        message = choice.get("delta")
    view: dict[str, JsonValue] = {
        "index": choice.get("index"),
        "finish_reason": choice.get("finish_reason"),
        "message": message_view(message),
    }
    return view


def choices_view(response: Choices) -> list[JsonValue]:
    views: list[JsonValue] = [choice_view(choice) for choice in response["choices"]]
    return views


def block_view(block: Bag) -> JsonValue:
    view: dict[str, JsonValue] = {
        "type": block.get("type"),
        "text": block.get("text"),
        "id": block.get("id"),
        "name": block.get("name"),
    }
    return view


def anthropic_view(response: dict[str, JsonValue]) -> JsonValue:
    typed = ANTHROPIC.validate_python(response)
    content = typed.get("content")
    blocks: list[JsonValue] = [] if content is None else [block_view(block) for block in content]
    view: dict[str, JsonValue] = {"content": blocks, "stop_reason": typed.get("stop_reason")}
    return view


def applied_guardrails(data: dict[str, JsonValue]) -> list[JsonValue] | None:
    fields = FIELDS.validate_python(data)
    bucket = fields.get("metadata")
    if "litellm_metadata" in data:
        bucket = fields.get("litellm_metadata")
    if bucket is None:
        return None
    names = bucket.get("applied_guardrails")
    if names is None:
        return None
    view: list[JsonValue] = list(names)
    return view


def dump_view(response: ModelResponse | ModelResponseStream) -> JsonValue:
    parsed = TypeAdapter(Choices).validate_json(response.model_dump_json())
    view: dict[str, JsonValue] = {"choices": choices_view(parsed)}
    return view


def blocked_from_success(
    guardrail: ToolPermissionGuardrail,
    data: dict[str, JsonValue],
    response: ModelResponse | dict[str, JsonValue],
) -> dict[str, JsonValue] | None:
    try:
        asyncio.run(guardrail.async_post_call_success_hook(data, UserAPIKeyAuth(), response))  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/tool_permission.py#L836-L890
    except GuardrailRaisedException as error:
        return blocked_of(error)
    return None


def success(input_value: Input) -> JsonValue:
    guardrail = build_guardrail(input_value)
    payload = input_value.get("response")
    if payload is None:
        raise ValueError("success mode needs a response")
    data = dict(input_value["data"])
    if input_value["format"] == "openai":
        response = ModelResponse(**payload)
        blocked = blocked_from_success(guardrail, data, response)
        response_view = dump_view(response)
    else:
        anthropic_response = dict(payload)
        blocked = blocked_from_success(guardrail, data, anthropic_response)
        response_view = anthropic_view(anthropic_response)
    answer: dict[str, JsonValue] = {
        "blocked": blocked,
        "response": response_view,
        "applied_guardrails": applied_guardrails(data),
    }
    return answer


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
            events.append(payload)
    view: dict[str, JsonValue] = {"events": events}
    return view


async def openai_source(chunks: list[dict[str, JsonValue]]) -> AsyncIterator[ModelResponseStream]:
    for chunk in chunks:
        yield ModelResponseStream(**chunk)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103-L2160


async def raw_source(chunks: list[str]) -> AsyncIterator[str]:
    for chunk in chunks:
        yield chunk


Streamed = ModelResponseStream | bytes | str


def streamed_view(item: Streamed) -> JsonValue:
    match item:
        case bytes():
            return raw_chunk_view(item.decode("utf-8"))
        case str():
            return raw_chunk_view(item)
        case ModelResponseStream():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103-L2160
            return dump_view(item)


async def streaming(input_value: Input) -> JsonValue:
    guardrail = build_guardrail(input_value)
    data = dict(input_value["data"])
    blocked: dict[str, JsonValue] | None = None
    if input_value["raw_chunks"]:
        source: AsyncIterator[ModelResponseStream] | AsyncIterator[str] = raw_source(
            input_value["raw_chunks"]
        )
    else:
        source = openai_source(input_value["chunks"])
    chunks: list[JsonValue] = []
    try:
        hook = guardrail.async_post_call_streaming_iterator_hook(UserAPIKeyAuth(), source, data)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/tool_permission.py#L892-L970
        chunks = [streamed_view(item) async for item in hook]
    except GuardrailRaisedException as error:
        blocked = blocked_of(error)
    answer: dict[str, JsonValue] = {
        "chunks": chunks,
        "blocked": blocked,
    }
    return answer


def run(input_value: Input) -> JsonValue:
    # LiteLLM prints its provider list to stdout, and the runner reads stdout as the answer
    with contextlib.redirect_stdout(io.StringIO()):
        if input_value["mode"] == "success":
            return success(input_value)
        return asyncio.run(streaming(input_value))
