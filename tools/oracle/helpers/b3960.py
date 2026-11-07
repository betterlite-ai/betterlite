import asyncio
import copy
import json
import os
from collections.abc import Awaitable, Callable, Mapping
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.integrations.argilla import ArgillaLogger
from litellm.integrations.custom_logger import CustomLogger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.integrations.argilla import ArgillaItem
from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]


class _Calls:
    def __init__(self, replies: list[Fields]) -> None:
        self.pending = list(replies)
        self.log: list[Json] = []

    def answer(
        self,
        method: str,
        url: str,
        headers: Mapping[str, str] | None,
        body: Json,
        timeout: int | None,
    ) -> httpx.Response:
        normalized: dict[str, Json] = {
            name.lower(): value for name, value in (headers or {}).items()
        }
        call: Fields = {
            "method": method,
            "url": url,
            "headers": normalized,
            "body": copy.deepcopy(body),
            "timeout": timeout,
        }
        self.log.append(call)
        reply = self.pending.pop(0)
        failure = reply.get("raise")
        if failure is not None:
            raise RuntimeError(str(failure))
        status = cast(int, reply["status"])
        text = cast(str, reply["body"])
        return httpx.Response(status, text=text, request=httpx.Request(method, url))

    def recorded(self) -> Json:
        return cast(Json, self.log)


class _ModuleClient:
    def __init__(self, calls: _Calls) -> None:
        self.calls = calls

    def get(self, url: str, headers: dict[str, str]) -> httpx.Response:
        return self.calls.answer("GET", url, headers, None, None)

    def post(
        self,
        url: str,
        json: Json = None,
        headers: dict[str, str] | None = None,
    ) -> httpx.Response:
        return self.calls.answer("POST", url, headers, json, None)


class _AsyncClient:
    def __init__(self, calls: _Calls) -> None:
        self.calls = calls

    async def put(
        self,
        url: str,
        data: str | None = None,
        headers: dict[str, str] | None = None,
        timeout: int | None = None,  # noqa: ASYNC109 - the fake mirrors the httpx client signature
    ) -> httpx.Response:
        body: Json = None if data is None else cast(Json, json.loads(data))
        return self.calls.answer("PUT", url, headers, body, timeout)


class _Draws:
    def __init__(self, values: list[float]) -> None:
        self.remaining = list(values)

    def __call__(self) -> float:
        return self.remaining.pop(0)


class _ScriptedHook(CustomLogger):
    def __init__(self, effect: str) -> None:
        self.effect = effect
        self.seen: list[Json] = []
        self.payload_present = False

    async def async_dataset_hook(
        self,
        logged_item: ArgillaItem,
        standard_logging_payload: StandardLoggingPayload | None,
    ) -> ArgillaItem | None:
        self.payload_present = standard_logging_payload is not None
        self.seen.append(cast(Json, logged_item))
        if self.effect == "not_implemented":
            raise NotImplementedError
        if self.effect == "failed":
            raise ValueError("the scripted dataset hook failed")
        if self.effect == "filter":
            return None
        return logged_item


def _optional_string(value: Json) -> str | None:
    return value if isinstance(value, str) else None


def _floats(value: Json) -> list[float]:
    if not isinstance(value, list):
        return []
    return [cast(float, item) for item in value]


def _strings(value: Json) -> list[str]:
    if not isinstance(value, list):
        return []
    return [cast(str, item) for item in value]


def _fields(value: Json) -> Fields:
    return cast(Fields, value) if isinstance(value, dict) else {}


def _kwargs(action: Fields) -> Fields:
    kwargs = _fields(action.get("kwargs"))
    return {"standard_logging_object": kwargs.get("standard_logging_object")}


def _chat_kind(error: Exception) -> str:
    text = str(error)
    if text.startswith("No chat messages found"):
        return "missing_chat_messages"
    if text.startswith("Invalid chat messages format"):
        return "invalid_chat_messages"
    return "unexpected_chat_error"


def _response_kind(error: Exception) -> str:
    text = str(error)
    if text.startswith("No response found"):
        return "missing_response"
    if isinstance(error, KeyError) and error.args == ("response",):
        return "missing_response"
    if text.startswith("Invalid response format"):
        return "invalid_response"
    return "unexpected_response_error"


async def _log_event(step: str, action: Fields, logger: ArgillaLogger) -> None:
    kwargs = _kwargs(action)
    if step == "sync_success":
        log_success = cast(
            Callable[[Fields, object, object, object], None],
            logger.log_success_event,
        )
        log_success(kwargs, None, None, None)
        return
    if step == "async_success":
        log_success_async = cast(
            Callable[[Fields, object, object, object], Awaitable[None]],
            logger.async_log_success_event,
        )
        await log_success_async(kwargs, None, None, None)
        return
    log_failure = cast(
        Callable[[Fields, object, object, object], Awaitable[None]],
        logger.async_log_failure_event,
    )
    await log_failure(kwargs, None, None, None)


def _extract(step: str, action: Fields, logger: ArgillaLogger) -> Json:
    payload = cast(StandardLoggingPayload, action.get("payload"))
    if step == "chat_messages":
        try:
            return {"messages": cast(Json, logger.get_chat_messages(payload))}
        except Exception as error:  # noqa: BLE001 - the case compares which extraction error is raised
            return {"raised": _chat_kind(error)}
    try:
        return {"response": cast(Json, logger.get_str_response(payload))}
    except Exception as error:  # noqa: BLE001 - the case compares which extraction error is raised
        return {"raised": _response_kind(error)}


async def _perform(action: Fields, logger: ArgillaLogger) -> Json:
    step = action.get("do")
    match step:
        case "sync_success" | "async_success" | "async_failure":
            await _log_event(step, action, logger)
            return None
        case "flush":
            before = len(cast("list[Json]", logger.log_queue))
            flush = cast(Callable[[], Awaitable[None]], logger.flush_queue)
            await flush()
            return {"batch": before}
        case "chat_messages" | "response":
            return _extract(step, action, logger)
        case _:
            message = f"unknown action: {step}"
            raise ValueError(message)


def _apply_batch_size(logger: ArgillaLogger, value: Json) -> None:
    if isinstance(value, int) and not isinstance(value, bool):
        logger.batch_size = value


async def _run_scenario(scenario: Fields) -> Json:
    calls = _Calls([_fields(reply) for reply in cast("list[Json]", scenario.get("replies") or [])])
    environment = cast("dict[str, str]", scenario.get("environment") or {})
    arguments = _fields(scenario.get("arguments"))
    hooks = [_ScriptedHook(effect) for effect in _strings(scenario.get("hooks"))]
    draws = _Draws(_floats(scenario.get("random")))
    with (
        patch.dict(os.environ, environment),
        patch.object(litellm, "module_level_client", _ModuleClient(calls)),
        patch.object(litellm, "callbacks", hooks),
        patch.object(
            litellm, "argilla_transformation_object", scenario.get("transformation_object")
        ),
        patch.object(litellm, "argilla_batch_size", None),
        patch("litellm.integrations.argilla.random.random", draws),
    ):
        try:
            logger = ArgillaLogger(
                argilla_api_key=_optional_string(arguments.get("api_key")),
                argilla_dataset_name=_optional_string(arguments.get("dataset_name")),
                argilla_base_url=_optional_string(arguments.get("base_url")),
            )
        except Exception:  # noqa: BLE001 - an initialization failure is what the validation case compares
            return {"initialized": False, "calls": calls.recorded()}
        logger.async_httpx_client = cast(AsyncHTTPHandler, _AsyncClient(calls))
        flush_interval = logger.flush_interval
        logger.flush_interval = 3600
        _apply_batch_size(logger, scenario.get("batch_size"))
        actions = [
            _fields(item) for item in cast("list[Json]", scenario.get("actions") or [])
        ]
        results = [await _perform(action, logger) for action in actions]
        hook_results: list[Json] = [
            {
                "effect": hook.effect,
                "seen": cast(Json, hook.seen),
                "payload_present": hook.payload_present,
            }
            for hook in hooks
        ]
        return {
            "initialized": True,
            "credentials": {
                "dataset_name": logger.default_credentials["ARGILLA_DATASET_NAME"],
                "base_url": logger.default_credentials["ARGILLA_BASE_URL"],
                "api_key": logger.default_credentials["ARGILLA_API_KEY"],
            },
            "sampling_rate": logger.sampling_rate,
            "batch_size": logger.batch_size,
            "flush_interval": flush_interval,
            "actions": cast(Json, results),
            "records": cast(Json, list(cast("list[Json]", logger.log_queue))),
            "calls": calls.recorded(),
            "hooks": cast(Json, hook_results),
        }


def run(input_value: dict[str, Json]) -> Json:
    scenarios = cast("list[dict[str, Json]]", input_value.get("scenarios") or [])
    results = [asyncio.run(_run_scenario(scenario)) for scenario in scenarios]
    return {"results": cast(Json, results)}
