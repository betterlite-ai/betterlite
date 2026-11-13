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
        extra = kwargs.get("additional_args")
        complete: Json = None
        if isinstance(extra, dict):
            complete = cast(Object, extra).get("complete_input_dict")
        self.events.append([
            cast(Json, kwargs.get("input")),
            cast(Json, kwargs.get("original_response")),
            complete,
        ])


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
        return {"model": model, "ok": False}
    return {"model": model, "ok": True, "base_model": base, "request_model": base or model}


def identity(call: Object) -> Json:
    host, base, key = text(call, "host"), opt(call, "api_base"), opt(call, "api_key")
    with patch.dict(os.environ, string_map(call, "env"), clear=True):
        resolved, resolved_key = INFO[host](base, key)
        if host == "clarifai":
            supported = cast(Json, list(CLARIFAI.get_supported_openai_params("")))
            models: list[Json] = [clarifai_model(model) for model in items(call, "models")]
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


def choice_value(choice: Object) -> Json:
    message = choice.get("message")
    fields = cast(Object, message) if isinstance(message, dict) else {}
    return [
        choice.get("index"),
        choice.get("finish_reason"),
        fields.get("role"),
        fields.get("content"),
        fields.get("tool_calls"),
    ]


def reply_value(result: ModelResponse) -> Object:
    dumped = cast(Object, result.model_dump())
    usage = dumped.get("usage")
    counts = cast(Object, usage) if isinstance(usage, dict) else {}
    choices = dumped.get("choices")
    picked: list[Json] = []
    if isinstance(choices, list):
        picked = [choice_value(cast(Object, choice)) for choice in cast("list[Json]", choices)]
    return {
        "kind": "reply",
        "object": dumped.get("object"),
        "model": dumped.get("model") or "",
        "usage": [
            counts.get(name) or 0
            for name in ("prompt_tokens", "completion_tokens", "total_tokens")
        ],
        "choices": picked,
    }


def error_value(error: Exception, supplied: dict[str, str]) -> Object:
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


def reply(call: Object) -> Json:
    host = text(call, "host")
    transform = TRANSFORM.get(host)
    if transform is None:
        raise ValueError(f"unknown reply host {host}")
    supplied = string_map(call, "headers")
    logger = Log()
    data = call.get("request_data")
    try:
        result = transform(
            model=opt(call, "model") or "",
            raw_response=httpx.Response(
                cast(int, call.get("status", 0)),
                text=opt(call, "body") or "",
                headers=dict(supplied),
            ),
            model_response=ModelResponse(),
            logging_obj=logger,
            request_data=cast(Object, data) if isinstance(data, dict) else {},
            messages=cast("list[AllMessageValues]", items(call, "messages")),
            optional_params={},
            litellm_params={},
            encoding=None,
            api_key=None,
            json_mode=bool(call.get("json_mode", False)),
        )
    except Exception as error:  # noqa: BLE001 - the error record is what the helper compares
        return {"events": logger.events, "outcome": error_value(error, supplied)}
    return {"events": logger.events, "outcome": reply_value(result)}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise TypeError("calls must be a list")
    results: list[Json] = []
    for call in cast("list[Json]", calls):
        fields = obj(call)
        kind = text(fields, "kind")
        if kind == "reply":
            results.append(reply(fields))
        elif kind == "identity":
            results.append(identity(fields))
        else:
            raise ValueError(f"unknown call kind {kind}")
    return results
