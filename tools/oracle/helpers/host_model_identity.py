import json
import os
from collections.abc import Callable, Mapping
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.clarifai.chat.transformation import ClarifaiConfig
from litellm.llms.compactifai.chat.transformation import CompactifAIChatConfig
from litellm.llms.datarobot.chat.transformation import DataRobotConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnknownArgumentType=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

CLARIFAI = ClarifaiConfig()
COMPACTIFAI = CompactifAIChatConfig()
DATAROBOT = DataRobotConfig()


class ProviderInfo(Protocol):
    def __call__(
        self, api_base: str | None, api_key: str | None
    ) -> tuple[str | None, str | None]: ...


class ApiError(Protocol):
    status_code: int
    message: str
    headers: Mapping[str, str] | None


CLARIFAI_TRANSFORM = cast(Callable[..., ModelResponse], CLARIFAI.transform_response)
COMPACTIFAI_TRANSFORM = cast(Callable[..., ModelResponse], COMPACTIFAI.transform_response)
CLARIFAI_INFO = cast(ProviderInfo, CLARIFAI._get_openai_compatible_provider_info)
COMPACTIFAI_INFO = cast(ProviderInfo, COMPACTIFAI._get_openai_compatible_provider_info)
DATAROBOT_INFO = cast(ProviderInfo, DATAROBOT._get_openai_compatible_provider_info)
DATAROBOT_COMPLETE = cast(Callable[..., str], DATAROBOT.get_complete_url)
CLARIFAI_ERROR = cast(Callable[[str, int, httpx.Headers], ApiError], CLARIFAI.get_error_class)
COMPACTIFAI_ERROR = cast(
    Callable[[str, int, httpx.Headers], ApiError], COMPACTIFAI.get_error_class
)


class FixtureResponse:
    def __init__(self, status_code: int, text: str, supplied: Mapping[str, str]) -> None:
        self.status_code = status_code
        self.text = text
        self.headers = httpx.Headers(dict(supplied))

    def json(self) -> object:
        return cast(object, json.loads(self.text))


class FixtureLogger:
    def __init__(self) -> None:
        self.events: list[Json] = []

    def post_call(self, **kwargs: object) -> None:
        additional = kwargs.get("additional_args")
        complete: Json = None
        if isinstance(additional, dict):
            complete = cast(Object, additional).get("complete_input_dict")
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


def string(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_string(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    return value if isinstance(value, str) else None


def string_map(fields: Object, name: str) -> dict[str, str]:
    entries = fields.get(name)
    if not isinstance(entries, dict):
        return {}
    return {
        key: value
        for key, value in cast(Object, entries).items()
        if isinstance(value, str)
    }


def model_list(fields: Object) -> list[str | None]:
    entries = fields.get("models")
    if not isinstance(entries, list):
        return []
    return [entry if isinstance(entry, str) else None for entry in cast("list[Json]", entries)]


def messages(fields: Object) -> list[AllMessageValues]:
    entries = fields.get("messages")
    if not isinstance(entries, list):
        return []
    return cast("list[AllMessageValues]", entries)


def request_data(fields: Object) -> Object:
    value = fields.get("request_data")
    return cast(Object, value) if isinstance(value, dict) else {}


def clarifai_model(model: str | None) -> Object:
    try:
        base = CLARIFAI.get_base_model(model)
    except Exception:  # noqa: BLE001 - a wrong arity is the outcome a caller branches on
        return {"model": model, "ok": False, "base_model": None, "request_model": None}
    return {"model": model, "ok": True, "base_model": base, "request_model": base or model}


def identity(step: Object) -> Json:
    host = string(step, "host")
    api_base = optional_string(step, "api_base")
    api_key = optional_string(step, "api_key")
    environment = string_map(step, "env")
    if host == "clarifai":
        with patch.dict(os.environ, environment, clear=True):
            base, key = CLARIFAI_INFO(api_base, api_key)
            supported = cast(Json, list(CLARIFAI.get_supported_openai_params("")))
        return {
            "host": host,
            "api_base": base,
            "api_key": key,
            "supported": supported,
            "models": [clarifai_model(model) for model in model_list(step)],
        }
    if host == "compactifai":
        with patch.dict(os.environ, environment, clear=True):
            base, key = COMPACTIFAI_INFO(api_base, api_key)
        return {"host": host, "api_base": base, "api_key": key}
    if host != "datarobot":
        raise ValueError(f"unknown identity host {host}")
    with patch.dict(os.environ, environment, clear=True):
        base, key = DATAROBOT_INFO(api_base, api_key)
        complete = DATAROBOT_COMPLETE(api_base, api_key, "", {}, {})
    return {"host": host, "api_base": base, "api_key": key, "complete_url": complete}


def reply_value(result: ModelResponse) -> Object:
    dumped = cast(Object, result.model_dump())
    usage = dumped.get("usage")
    counts = cast(Object, usage) if isinstance(usage, dict) else {}
    choices = dumped.get("choices")
    return {
        "object": dumped.get("object"),
        "model": dumped.get("model") or "",
        "usage": {
            "prompt_tokens": counts.get("prompt_tokens") or 0,
            "completion_tokens": counts.get("completion_tokens") or 0,
            "total_tokens": counts.get("total_tokens") or 0,
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


def error_outcome(error: Exception, supplied: Mapping[str, str]) -> Object:
    status = cast(object, getattr(error, "status_code", None))
    carried = cast(object, getattr(error, "headers", None))
    reported: Json = None
    if isinstance(carried, httpx.Headers):
        reported = {name: carried.get(name) for name in supplied}
    return {
        "kind": "error",
        "status": status if isinstance(status, int) else None,
        "headers": reported,
    }


def reply(step: Object) -> Json:
    host = string(step, "host")
    if host == "clarifai":
        transform = CLARIFAI_TRANSFORM
    elif host == "compactifai":
        transform = COMPACTIFAI_TRANSFORM
    else:
        raise ValueError(f"unknown reply host {host}")
    supplied = string_map(step, "headers")
    logger = FixtureLogger()
    try:
        result = transform(
            model=string(step, "model"),
            raw_response=FixtureResponse(
                cast(int, step.get("status", 200)), string(step, "body"), supplied
            ),
            model_response=ModelResponse(),
            logging_obj=logger,
            request_data=request_data(step),
            messages=messages(step),
            optional_params={},
            litellm_params={},
            encoding=None,
            api_key=optional_string(step, "api_key"),
            json_mode=bool(step.get("json_mode", False)),
        )
    except Exception as error:  # noqa: BLE001 - the error record is what the helper compares
        return {
            "host": host,
            "events": logger.events,
            "outcome": error_outcome(error, supplied),
        }
    return {
        "host": host,
        "events": logger.events,
        "outcome": {"kind": "reply", "reply": reply_value(result)},
    }


def error_class(step: Object) -> Json:
    host = string(step, "host")
    if host == "clarifai":
        build = CLARIFAI_ERROR
    elif host == "compactifai":
        build = COMPACTIFAI_ERROR
    else:
        raise ValueError(f"unknown error class host {host}")
    supplied = string_map(step, "headers")
    error = build(
        string(step, "message"),
        cast(int, step.get("status", 500)),
        httpx.Headers(dict(supplied)),
    )
    carried: Json = None
    if error.headers is not None:
        carried = {name: error.headers.get(name) for name in supplied}
    return {
        "host": host,
        "status": error.status_code,
        "message": error.message,
        "headers": carried,
    }


def run(input_value: dict[str, Json]) -> Json:
    steps = input_value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    results: list[Json] = []
    for step in cast("list[Json]", steps):
        fields = obj(step)
        kind = string(fields, "kind")
        if kind == "identity":
            results.append(identity(fields))
        elif kind == "reply":
            results.append(reply(fields))
        elif kind == "error_class":
            results.append(error_class(fields))
        else:
            raise ValueError(f"unknown step kind {kind}")
    return results
