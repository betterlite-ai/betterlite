from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.cohere.chat.v2_transformation import CohereV2ChatConfig
from litellm.types.utils import ModelResponse, Usage

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging
    from litellm.types.llms.openai import AllMessageValues

# ruff: noqa: SLF001
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
    config = CohereV2ChatConfig()
    model = text(fields, "supported_model")
    supported: list[Json] = cast(list[Json], list(config.get_supported_openai_params(model)))
    mapped: list[Json] = []
    for entry in listing(fields, "mappings"):
        mapping = obj(entry)
        mapped.append(cast(Json, config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]
            obj(mapping.get("non_default_params", {})), obj(mapping.get("optional_params", {})),
            model, drop_params=False)))
    return {"supported": supported, "mapped": mapped}


def annotations(fields: Object) -> Json:
    try:
        found = CohereV2ChatConfig()._translate_citations_to_openai_annotations(  # pyright: ignore[reportUnknownMemberType]
            cast("list[dict[str, object]]", listing(fields, "citations")))
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    return {"ok": True, "value": cast(Json, found)}


def tool_calls(message: ModelResponse) -> Json:
    calls = message.choices[0].message.tool_calls
    if calls is None:
        return None
    # model_dump carries index, id, type and function exactly as the pinned message stores them
    return cast(Json, [call.model_dump() for call in calls])  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]


def projection(result: ModelResponse, initial_id: str) -> Json:
    choice = result.choices[0]
    usage = cast(Usage, result.usage)  # pyright: ignore[reportAttributeAccessIssue]
    return {
        "created": result.created,
        "model": result.model,
        "finish_reason": choice.finish_reason,
        "content": choice.message.content,
        "tool_calls": tool_calls(result),
        "annotations": cast(Json, getattr(choice.message, "annotations", None)),
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
        "id_preserved": result.id == initial_id,
    }


def reply_request(fields: Object) -> Json:
    body = text(fields, "body")
    status = integer(fields, "status")
    model = text(fields, "model")
    created = integer(fields, "created")
    response = ModelResponse(id=text(fields, "id"))
    response.choices[0].finish_reason = cast(  # pyright: ignore[reportAttributeAccessIssue]
        str | None, fields.get("finish_reason"))
    message = obj(fields.get("message", {}))
    response.choices[0].message.content = cast(str | None, message.get("content"))
    initial_id = response.id
    try:
        clock = "litellm.llms.cohere.chat.v2_transformation.time.time"
        with patch(clock, return_value=float(created)):
            result = CohereV2ChatConfig().transform_response(  # pyright: ignore[reportUnknownMemberType]
                model=model, raw_response=httpx.Response(status, text=body),
                model_response=response, logging_obj=cast("Logging", None), request_data={},
                messages=cast("list[AllMessageValues]", []), optional_params={},
                litellm_params={}, encoding=None)
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    return {"ok": True, "value": projection(result, initial_id)}


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "options": options,
    "annotations": annotations,
    "reply": lambda f: [reply_request(obj(request)) for request in listing(f, "requests")],
}


def run(argument: Json) -> Json:
    fields = obj(argument)
    handler = HANDLERS.get(text(fields, "function"))
    if handler is None:
        raise ValueError("unknown cohere_v2_reply function")
    return handler(fields)
