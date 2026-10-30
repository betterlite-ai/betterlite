import json
from typing import NotRequired, TypeAlias, TypedDict, cast

from litellm.anthropic_interface.exceptions.exception_mapping_utils import (
    AnthropicErrorSseFrame,
    AnthropicExceptionMapping,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    action: str
    status: NotRequired[int]
    raw_message: NotRequired[str]
    request_id: NotRequired[str | None]
    call_id: NotRequired[str | None]
    bytes: NotRequired[list[int]]


def run(argument: Input) -> object:
    action = argument["action"]
    status = argument.get("status", 0)
    raw_message = argument.get("raw_message", "")
    if action == "transform":
        return AnthropicExceptionMapping.transform_to_anthropic_error(
            status_code=status,
            raw_message=raw_message,
            request_id=argument.get("request_id"),
        )
    if action == "frame":
        frame = AnthropicErrorSseFrame(
            status,
            AnthropicExceptionMapping.transform_to_anthropic_error(status, raw_message),
        )
        return {
            "status": frame.status_code,
            "body": str(frame),
            "json_body": frame.json_body(argument.get("call_id")),
        }
    if action == "wire_roundtrip":
        return cast(Json, json.loads(bytes(argument.get("bytes", []))))
    raise ValueError("unknown messages_errors action")
