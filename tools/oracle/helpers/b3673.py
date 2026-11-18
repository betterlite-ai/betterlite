import time
import uuid
from functools import partial
from typing import Final, Literal, TypedDict

import litellm
from fastapi import HTTPException
from litellm.exceptions import ModifyResponseException
from litellm.llms.anthropic.chat.guardrail_translation.handler import (
    AnthropicMessagesHandler,
)
from litellm.llms.openai.responses.guardrail_translation.handler import (
    OpenAIResponsesHandler,
)
from litellm.types.llms.openai import ErrorEvent
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ERROR_EVENTS: Final[TypeAdapter[list[ErrorEvent]]] = TypeAdapter(list[ErrorEvent])


class Stamp(TypedDict):
    response_id: str
    message_id: str
    created_at: int


class Input(TypedDict):
    operation: Literal[
        "responses_standalone",
        "responses_continuation",
        "responses_open_item",
        "responses_stream_error",
        "anthropic_standalone",
        "anthropic_continuation",
        "anthropic_observation",
        "anthropic_scan_key",
    ]
    message: str
    model: str
    original_response: JsonValue | None
    stamp: Stamp
    events: list[JsonValue]
    chunks: list[str]
    status_code: int
    message_id: str


def response_frames(input_value: Input, *, stream_started: bool) -> JsonValue:
    request: dict[str, JsonValue] = {}
    error = ModifyResponseException(
        message=input_value["message"],
        model=input_value["model"],
        request_data=request,
        original_response=input_value["original_response"],
    )
    chunks = OpenAIResponsesHandler().build_block_sse_chunks(
        error, stream_started=stream_started, responses_so_far=input_value["events"]
    )
    return {
        "frames": [
            JSON.validate_json(chunk.decode("utf-8").removeprefix("data: ").strip())
            for chunk in chunks
        ]
    }


def error_items(input_value: Input) -> JsonValue:
    error = HTTPException(
        status_code=input_value["status_code"], detail=input_value["message"]
    )
    items = OpenAIResponsesHandler().build_stream_error_items(error, input_value["events"])
    if items is None:
        return {"items": []}
    typed_items = ERROR_EVENTS.validate_python(items)
    return {"items": [JSON.validate_json(item.model_dump_json()) for item in typed_items]}


def anthropic_frames(input_value: Input) -> JsonValue:
    handler = AnthropicMessagesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L302-L304
    request: dict[str, JsonValue] = {}
    error = ModifyResponseException(
        message=input_value["message"], model=input_value["model"],
        request_data=request, original_response=input_value["original_response"],
    )
    chunks = handler.build_block_sse_chunks(
        error,
        stream_started=input_value["operation"] == "anthropic_continuation",
        responses_so_far=[chunk.encode("utf-8") for chunk in input_value["chunks"]],
    )
    return {
        "frames": [
            {
                "event": chunk.decode("utf-8").splitlines()[0][7:],
                "data": JSON.validate_json(chunk.decode("utf-8").splitlines()[1][6:]),
            }
            for chunk in chunks
        ]
    }


def observation(input_value: Input) -> JsonValue:
    handler = AnthropicMessagesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L302-L304
    chunks = [chunk.encode("utf-8") for chunk in input_value["chunks"]]
    key = handler.get_streaming_scan_key(chunks)
    if input_value["operation"] == "anthropic_observation":
        text = handler.get_streaming_string_so_far(chunks)
        ended = key.stream_ended if key is not None else False
        return {"text": text, "ended": ended}
    if key is None:
        return {"scan_key": None}
    return {
        "scan_key": {
            "texts": list(key.texts),
            "tool_calls": [JSON.validate_json(text) for text in key.tool_calls],
            "stream_ended": key.stream_ended,
            "tool_calls_in_flight": key.tool_calls_in_flight,
        }
    }


def answer(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "responses_standalone":
            return response_frames(input_value, stream_started=False)
        case "responses_continuation" | "responses_open_item":
            return response_frames(input_value, stream_started=True)
        case "responses_stream_error":
            return error_items(input_value)
        case "anthropic_standalone" | "anthropic_continuation":
            return anthropic_frames(input_value)
        case "anthropic_observation" | "anthropic_scan_key":
            return observation(input_value)


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    stamp = input_value["stamp"]
    ids = [uuid.UUID(stamp["message_id"].removeprefix("msg_"))]
    if input_value["operation"].startswith("responses_"):
        ids.insert(0, uuid.UUID(stamp["response_id"].removeprefix("resp_")))
    saved_uuid = uuid.uuid4
    saved_time = time.time
    uuid.uuid4 = iter(ids).__next__
    time.time = partial(float, stamp["created_at"])
    try:
        return answer(input_value)
    finally:
        uuid.uuid4 = saved_uuid
        time.time = saved_time
