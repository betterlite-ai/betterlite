import os
from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.clarifai.chat.transformation import ClarifaiConfig
from litellm.llms.compactifai.chat.transformation import CompactifAIChatConfig
from litellm.llms.datarobot.chat.transformation import DataRobotConfig
from litellm.types.utils import ModelResponse

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnknownArgumentType=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

CLARIFAI = ClarifaiConfig()
COMPACTIFAI = CompactifAIChatConfig()
DATAROBOT = DataRobotConfig()
INFO = cast("dict[str, Callable[..., tuple[str | None, str | None]]]", {
    "clarifai": CLARIFAI._get_openai_compatible_provider_info,
    "compactifai": COMPACTIFAI._get_openai_compatible_provider_info,
    "datarobot": DATAROBOT._get_openai_compatible_provider_info,
})
TRANSFORM = cast("dict[str, Callable[..., ModelResponse]]", {
    "clarifai": CLARIFAI.transform_response,
    "compactifai": COMPACTIFAI.transform_response,
})
COMPLETE = cast(Callable[..., str], DATAROBOT.get_complete_url)


class Log:
    def __init__(self) -> None:
        self.events: list[Json] = []

    def post_call(self, **kwargs: object) -> None:
        extra = cast(object, kwargs.get("additional_args"))
        complete: Json = None
        if isinstance(extra, dict):
            complete = cast(Object, extra).get("complete_input_dict")
        self.events.append({
            "name": "post_call",
            "input": cast(Json, kwargs.get("input")),
            "api_key": cast(Json, kwargs.get("api_key")),
            "original_response": cast(Json, kwargs.get("original_response")),
            "complete_input_dict": complete,
        })


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return cast(Object, value)


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def opt(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    return value if isinstance(value, str) else None


def string_map(fields: Object, name: str) -> dict[str, str]:
    entries = fields.get(name)
    if not isinstance(entries, dict):
        return {}
    return {key: value for key, value in cast(Object, entries).items() if isinstance(value, str)}


def items(fields: Object, name: str) -> list[Json]:
    value = fields.get(name)
    return cast("list[Json]", value) if isinstance(value, list) else []


def clarifai_model(model: Json) -> Object:
    try:
        base = CLARIFAI.get_base_model(model if isinstance(model, str) else None)
    except Exception:  # noqa: BLE001 - a wrong arity is the outcome a caller branches on
        return {"model": model, "ok": False, "base_model": None, "request_model": None}
    return {"model": model, "ok": True, "base_model": base, "request_model": base or model}


def identity(step: Object) -> Json:
    host = text(step, "host")
    base = opt(step, "api_base")
    key = opt(step, "api_key")
    with patch.dict(os.environ, string_map(step, "env"), clear=True):
        resolved, resolved_key = INFO[host](base, key)
        if host == "clarifai":
            supported = cast(Json, list(CLARIFAI.get_supported_openai_params("")))
            models: list[Json] = [clarifai_model(model) for model in items(step, "models")]
            return {
                "host": host,
                "api_base": resolved,
                "api_key": resolved_key,
                "supported": supported,
                "models": models,
            }
        if host == "compactifai":
            return {"host": host, "api_base": resolved, "api_key": resolved_key}
        return {
            "host": host,
            "api_base": resolved,
            "api_key": resolved_key,
            "complete_url": COMPLETE(base, key, "", {}, {}),
        }


def reply_value(result: ModelResponse) -> Object:
    dumped = cast(Object, result.model_dump())
    usage = dumped.get("usage")
    counts = cast(Object, usage) if isinstance(usage, dict) else {}
    choices = dumped.get("choices")
    return {
        "object": dumped.get("object"),
        "model": dumped.get("model") or "",
        "usage": {
            name: counts.get(name) or 0
            for name in ("prompt_tokens", "completion_tokens", "total_tokens")
        },
        "choices": [choice_value(choice) for choice in cast("list[Json]", choices)]
        if isinstance(choices, list)
        else [],
    }


def choice_value(choice: Json) -> Object:
    fields = obj(choice)
    message = fields.get("message")
    message_fields = cast(Object, message) if isinstance(message, dict) else {}
    return {
        "index": fields.get("index"),
        "finish_reason": fields.get("finish_reason"),
        "message": {
            "role": message_fields.get("role"),
            "content": message_fields.get("content"),
            "tool_calls": message_fields.get("tool_calls"),
        },
    }


def error_outcome(error: Exception, supplied: dict[str, str]) -> Object:
    status = cast(object, getattr(error, "status_code", None))
    carried = cast(object, getattr(error, "headers", None))
    headers: Json = None
    if isinstance(carried, httpx.Headers):
        headers = {name: carried.get(name) for name in supplied}
    return {
        "kind": "error",
        "status": status if isinstance(status, int) else None,
        "headers": headers,
    }


def reply(step: Object) -> Json:
    host = text(step, "host")
    transform = TRANSFORM.get(host)
    if transform is None:
        raise ValueError(f"unknown reply host {host}")
    supplied = string_map(step, "headers")
    logger = Log()
    data = step.get("request_data")
    try:
        result = transform(
            model=text(step, "model"),
            raw_response=httpx.Response(
                cast(int, step.get("status", 200)),
                text=text(step, "body"),
                headers=dict(supplied),
            ),
            model_response=ModelResponse(),
            logging_obj=logger,
            request_data=cast(Object, data) if isinstance(data, dict) else {},
            messages=cast("list[AllMessageValues]", items(step, "messages")),
            optional_params={},
            litellm_params={},
            encoding=None,
            api_key=opt(step, "api_key"),
            json_mode=bool(step.get("json_mode", False)),
        )
    except Exception as error:  # noqa: BLE001 - the error record is what the helper compares
        return {"host": host, "events": logger.events, "outcome": error_outcome(error, supplied)}
    return {
        "host": host,
        "events": logger.events,
        "outcome": {"kind": "reply", "reply": reply_value(result)},
    }


def run(input_value: dict[str, Json]) -> Json:
    steps = input_value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    results: list[Json] = []
    for step in cast("list[Json]", steps):
        fields = obj(step)
        results.append(identity(fields) if text(fields, "kind") == "identity" else reply(fields))
    return results
