import json
from typing import NotRequired, TypeAlias, TypedDict, cast

from litellm.anthropic_interface.exceptions.exception_mapping_utils import (
    AnthropicErrorSseFrame,
    AnthropicExceptionMapping,
)
from litellm.types.llms.anthropic_messages.anthropic_request import AnthropicMetadata

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Action(TypedDict):
    action: str
    status: NotRequired[int]
    raw_message: NotRequired[str]
    request_id: NotRequired[str | None]
    call_id: NotRequired[str | None]
    json: NotRequired[str]
    metadata: NotRequired[bool]


class Batch(TypedDict):
    actions: list[Action]


def run(argument: Action | Batch) -> object:
    if "actions" in argument:
        return [run(action) for action in argument["actions"]]
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
        if argument.get("metadata", False):
            return AnthropicMetadata.model_validate_json(argument.get("json", "{}")).model_dump()
        return cast(Json, json.loads(argument.get("json", "{}")))
    raise ValueError("unknown messages_errors action")
