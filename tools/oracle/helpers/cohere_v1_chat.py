from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.cohere.chat.transformation import CohereChatConfig
from litellm.types.utils import ModelResponse, Usage

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging
    from litellm.types.llms.openai import AllMessageValues

# pyright: reportPrivateUsage=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def integer(fields: Object, name: str) -> int:
    value = fields.get(name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    return value


def listing(fields: Object, name: str) -> list[Json]:
    value = fields.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return value


def options(fields: Object) -> Json:
    instance = CohereChatConfig()
    model = text(fields, "supported_model")
    supported = cast("list[Json]", list(instance.get_supported_openai_params(model)))  # pyright: ignore[reportUnknownMemberType]
    mapped: list[Json] = []
    for entry in listing(fields, "mappings"):
        mapping = obj(entry)
        mapped.append(cast(Json, instance.map_openai_params(  # pyright: ignore[reportUnknownMemberType]
            obj(mapping.get("non_default_params", {})), obj(mapping.get("optional_params", {})),
            model, drop_params=False)))
    return {"supported": supported, "mapped": mapped}


def request(fields: Object) -> Json:
    return [request_outcome(obj(entry)) for entry in listing(fields, "requests")]


def request_outcome(fields: Object) -> Json:
    try:
        settings = obj(fields.get("settings", {}))
        instance = CohereChatConfig(**cast("dict[str, object]", settings))  # pyright: ignore[reportArgumentType]
        model = text(fields, "model")
        mapped = cast("dict[str, object]", instance.map_openai_params(  # pyright: ignore[reportUnknownMemberType]
            obj(fields.get("non_default_params", {})), obj(fields.get("optional_params", {})),
            model, drop_params=False))
        result = cast("dict[str, object]", instance.transform_request(  # pyright: ignore[reportUnknownMemberType]
            model=model, messages=cast("list[AllMessageValues]", listing(fields, "messages")),
            optional_params=mapped, litellm_params={}, headers={}))
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    return {"ok": True, "value": cast(Json, result)}


def reply(fields: Object) -> Json:
    return [reply_outcome(obj(entry)) for entry in listing(fields, "requests")]


def reply_outcome(fields: Object) -> Json:
    body = text(fields, "body")
    status = integer(fields, "status")
    model = text(fields, "model")
    created = integer(fields, "created")
    response = ModelResponse(id=text(fields, "id"))
    response.choices[0].finish_reason = cast(  # pyright: ignore[reportAttributeAccessIssue]
        "str | None", fields.get("finish_reason"))
    initial_id = response.id
    try:
        clock = "litellm.llms.cohere.chat.transformation.time.time"
        with patch(clock, return_value=float(created)):
            result = CohereChatConfig().transform_response(  # pyright: ignore[reportUnknownMemberType]
                model=model, raw_response=httpx.Response(status, text=body),
                model_response=response, logging_obj=cast("Logging", None), request_data={},
                messages=cast("list[AllMessageValues]", []), optional_params={},
                litellm_params={}, encoding=None)
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    return {"ok": True, "value": projection(result, initial_id)}


def projection(result: ModelResponse, initial_id: str) -> Json:
    choice = result.choices[0]
    usage = cast(Usage, result.usage)  # pyright: ignore[reportAttributeAccessIssue]
    calls = choice.message.tool_calls
    return {
        "created": result.created,
        "model": result.model,
        "finish_reason": choice.finish_reason,
        "content": choice.message.content,
        # model_dump carries id, type and function exactly as the pinned message stores them
        "tool_calls": None if calls is None else cast(Json, [call.model_dump() for call in calls]),  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]
        "citations": cast(Json, getattr(result, "citations", None)),
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
        "id_preserved": result.id == initial_id,
    }


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "options": options, "request": request, "reply": reply,
}


def run(argument: Json) -> Json:
    fields = obj(argument)
    handler = HANDLERS.get(text(fields, "function"))
    if handler is None:
        raise ValueError("unknown cohere_v1_chat function")
    return handler(fields)
