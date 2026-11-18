import uuid
from typing import Final, Literal, TypedDict
from unittest.mock import patch

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
litellm.suppress_debug_info = True


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


def response_frames(
    handler: OpenAIResponsesHandler, input_value: Input, stream_started: bool
) -> dict[str, list[JsonValue]]:
    stamp = input_value["stamp"]
    unique_id = uuid.UUID(stamp["response_id"].removeprefix("resp_"))
    error = ModifyResponseException(
        message=input_value["message"],
        model=input_value["model"],
        request_data={},
        original_response=input_value["original_response"],
    )
    with patch(
        "litellm.llms.openai.responses.guardrail_translation.handler.uuid.uuid4",
        return_value=unique_id,
    ), patch(
        "litellm.llms.openai.responses.guardrail_translation.handler.time.time",
        return_value=stamp["created_at"],
    ):
        chunks = handler.build_block_sse_chunks(
            error,
            stream_started=stream_started,
            responses_so_far=input_value["events"],
        )
    return {
        "frames": [
            JSON.validate_json(chunk.decode("utf-8").removeprefix("data: ").strip())
            for chunk in chunks
        ]
    }


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    if operation == "responses_standalone":
        return response_frames(OpenAIResponsesHandler(), input_value, False)
    if operation in ("responses_continuation", "responses_open_item"):
        return response_frames(OpenAIResponsesHandler(), input_value, True)
    if operation == "responses_stream_error":
        handler = OpenAIResponsesHandler()
        error = HTTPException(
            status_code=input_value["status_code"], detail=input_value["message"]
        )
        items = handler.build_stream_error_items(error, input_value["events"])
        if items is None:
            return {"items": []}
        typed_items = ERROR_EVENTS.validate_python(items)
        return {
            "items": [
                JSON.validate_json(item.model_dump_json()) for item in typed_items
            ]
        }
    if operation in ("anthropic_standalone", "anthropic_continuation"):
        handler = AnthropicMessagesHandler()
        error = ModifyResponseException(
            message=input_value["message"],
            model=input_value["model"],
            request_data={},
            original_response=input_value["original_response"],
        )
        unique_id = uuid.UUID(input_value["stamp"]["message_id"].removeprefix("msg_"))
        with patch("uuid.uuid4", return_value=unique_id):
            chunks = handler.build_block_sse_chunks(
                error,
                stream_started=operation == "anthropic_continuation",
                responses_so_far=[
                    chunk.encode("utf-8") for chunk in input_value["chunks"]
                ],
            )
        return {
            "frames": [
                {
                    "event": chunk.decode("utf-8").splitlines()[0][7:],
                    "data": JSON.validate_json(
                        chunk.decode("utf-8").splitlines()[1][6:]
                    ),
                }
                for chunk in chunks
            ]
        }
    handler = AnthropicMessagesHandler()
    chunks = input_value["chunks"]
    byte_chunks = [chunk.encode("utf-8") for chunk in chunks]
    if operation == "anthropic_observation":
        text = handler.get_streaming_string_so_far(byte_chunks)
        key = handler.get_streaming_scan_key(byte_chunks)
        ended = key.stream_ended if key is not None else False
        return {"text": text, "ended": ended}
    if operation == "anthropic_scan_key":
        key = handler.get_streaming_scan_key(byte_chunks)
        if key is None:
            return {"scan_key": None}
        return {
            "scan_key": {
                "texts": list(key.texts),
                "tool_calls": list(key.tool_calls),
                "stream_ended": key.stream_ended,
                "tool_calls_in_flight": key.tool_calls_in_flight,
            }
        }
    key = handler.get_streaming_scan_key(byte_chunks)
    if key is None:
        return {"scan_key": None}
    return {
        "scan_key": {
            "texts": list(key.texts),
            "tool_calls": list(key.tool_calls),
            "stream_ended": key.stream_ended,
            "tool_calls_in_flight": key.tool_calls_in_flight,
        }
    }
