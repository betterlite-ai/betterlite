import asyncio
import logging
import os
import uuid
from collections.abc import Awaitable, Callable, Mapping
from datetime import datetime, timedelta, timezone
from itertools import count
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm._logging import verbose_logger
from litellm.integrations.galileo import GalileoObserve
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def _header_names(headers: Mapping[str, Json]) -> list[Json]:
    return [cast(Json, name) for name in sorted(headers)]


class ScriptedHandler:
    def __init__(self, replies: list[Fields]) -> None:
        self.replies = replies
        self.calls: list[Json] = []

    def answer(
        self,
        method: str,
        url: str,
        headers: Mapping[str, Json],
        form: "Fields | None",
        payload: "Fields | None",
    ) -> httpx.Response:
        call: Fields = {
            "method": method,
            "url": url,
            "headers": _header_names(headers),
            "form": form,
            "payload": payload,
        }
        self.calls.append(call)
        if not self.replies:
            message = f"request to {url} failed: no scripted reply"
            raise RuntimeError(message)
        reply = self.replies.pop(0)
        return httpx.Response(
            status_code=cast(int, reply["status"]),
            text=cast(str, reply["body"]),
            request=httpx.Request(method, url),
        )

    async def get(self, url: str, headers: Fields) -> httpx.Response:
        return self.answer("GET", url, headers, None, None)

    async def post(
        self,
        url: str,
        headers: Fields,
        data: Fields | None = None,
        json: Fields | None = None,
    ) -> httpx.Response:
        payload = None if json is None else _payload_view(json)
        return self.answer("POST", url, headers, data, payload)


def _payload_view(payload: Fields) -> Fields:
    traces = cast(list[Fields], payload["traces"])
    ids: list[Json] = [
        [trace["id"], cast(list[Fields], trace["spans"])[0]["id"]] for trace in traces
    ]
    rest = {name: value for name, value in payload.items() if name != "traces"}
    return {"trace_ids": ids, "rest": rest}


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _kwargs(raw: Fields) -> Fields:
    optional_params: Fields = {}
    functions = raw.get("functions")
    if functions is not None:
        optional_params["functions"] = functions
    tools = raw.get("tools")
    if tools is not None:
        optional_params["tools"] = tools
    return {
        "standard_logging_object": raw.get("standard_logging_object"),
        "call_type": raw.get("call_type"),
        "model": raw.get("model"),
        "messages": raw.get("messages"),
        "input": raw.get("input"),
        "optional_params": optional_params,
    }


def _response(raw: "Fields | None") -> object:
    if raw is None:
        return None
    family = _text(raw.get("family"), "family")
    if family == "chat":
        return litellm.ModelResponse(choices=[{"message": raw.get("message")}])
    if family == "pass_through_dict":
        return cast("dict[str, Json]", raw.get("value") or {})
    message = f"unsupported response family: {family}"
    raise ValueError(message)


def _moment(value: "str | None") -> datetime | None:
    return None if value is None else datetime.fromisoformat(value)


def _record_view(record: Mapping[str, object]) -> Fields:
    return {name: cast(Json, value) for name, value in record.items() if value is not None}


def _prefill(logger: GalileoObserve, action: Fields) -> None:
    for index in range(cast(int, action.get("count", 0))):
        logger.in_memory_records.append({"input_text": str(index), "output_text": "out"})


async def _perform(
    logger: GalileoObserve,
    actions: list[Fields],
    offset_minutes: int,
) -> None:
    local_offset = timezone(timedelta(minutes=offset_minutes))
    log_success = cast(
        Callable[[Fields, object, "datetime | None", "datetime | None"], Awaitable[None]],
        logger.async_log_success_event,
    )
    log_failure = cast(
        Callable[[Fields, object, "datetime | None", "datetime | None"], Awaitable[None]],
        logger.async_log_failure_event,
    )
    for action in actions:
        step = _text(action.get("do"), "do")
        if step == "prefill":
            _prefill(logger, action)
            continue
        kwargs = _kwargs(cast(Fields, action.get("kwargs") or {}))
        if step == "success":
            response = _response(cast("Fields | None", action.get("response")))
            start_time = _moment(cast("str | None", action.get("start_time")))
            end_time = _moment(cast("str | None", action.get("end_time")))
            local_zone = staticmethod(lambda: local_offset)
            with patch.object(GalileoObserve, "_local_timezone", local_zone):
                await log_success(kwargs, response, start_time, end_time)
        elif step == "failure":
            await log_failure(kwargs, None, None, None)
        else:
            message = f"unknown action: {step}"
            raise ValueError(message)


def run(input_value: Fields) -> Json:
    environment = cast(dict[str, str], input_value.get("environment") or {})
    replies = cast(list[Fields], input_value.get("replies") or [])
    actions = cast(list[Fields], input_value["actions"])
    batch_size = cast(int, input_value.get("batch_size", 1))
    offset_minutes = cast(int, input_value.get("offset_minutes", 0))
    ids = count(1)
    with patch.dict(os.environ):
        for name in [name for name in os.environ if name.startswith("GALILEO_")]:
            del os.environ[name]
        os.environ.update(environment)
        logger = GalileoObserve()
    logger.batch_size = batch_size
    handler = ScriptedHandler(replies)
    logger.async_httpx_handler = cast(AsyncHTTPHandler, handler)
    recorded = Warnings()
    verbose_logger.addHandler(recorded)
    previous = verbose_logger.level
    verbose_logger.setLevel(logging.WARNING)
    try:
        with patch.object(uuid, "uuid4", lambda: uuid.UUID(int=next(ids))):
            asyncio.run(_perform(logger, actions, offset_minutes))
    finally:
        verbose_logger.removeHandler(recorded)
        verbose_logger.setLevel(previous)
    records: list[Json] = [
        cast(Json, _record_view(record)) for record in logger.in_memory_records
    ]
    result: Fields = {
        "records": records,
        "calls": handler.calls,
        "warnings": cast(Json, recorded.messages),
    }
    return result
