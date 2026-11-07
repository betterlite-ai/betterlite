import asyncio
import copy
import json
import logging
import os
from collections.abc import Awaitable, Sequence
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm._logging import verbose_logger
from litellm.integrations.rubrik import RubrikLogger
from litellm.llms.custom_httpx.http_handler import (
    AsyncHTTPHandler,
    _default_cached_client_timeout,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.guardrails import GuardrailEventHooks

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]


class InputError(ValueError):
    pass


def object_value(value: Json, name: str) -> Fields:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def number_value(value: Json, name: str) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise InputError(f"{name} must be a number")
    return float(value)


def integer_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise InputError(f"{name} must be a boolean")
    return value


class Script:
    def __init__(self, replies: Sequence[Json]) -> None:
        self.replies = list(replies)
        self.calls: list[Fields] = []
        self.logger: RubrikLogger | None = None

    def next_reply(self) -> Fields:
        if not self.replies:
            raise InputError("the scripted transport has no reply")
        return object_value(self.replies.pop(0), "reply")

    def push_during_send(self, reply: Fields) -> None:
        during = reply.get("during_send")
        if during is None:
            return
        logger = self.logger
        if logger is None:
            raise InputError("the scripted transport is not attached")
        cast("list[Json]", logger.log_queue).append(during)


class ScriptedHandler:
    def __init__(self, script: Script, tier: str) -> None:
        self.script = script
        self.tier = tier
        self.client_timeout: httpx.Timeout | None = None

    def effective_timeout(self, override: object) -> Json:
        if self.tier == "logging":
            return None
        if override is not None:
            seconds = number_value(cast(Json, override), "timeout")
            return {"total": seconds, "connect": seconds}
        client_timeout = self.client_timeout
        if client_timeout is None:
            raise InputError("the moderation client timeout is unknown")
        return {"total": client_timeout.read, "connect": client_timeout.connect}

    def record(self, url: str, options: dict[str, object]) -> None:
        headers = cast("dict[str, str] | None", options.get("headers"))
        pairs: list[Json] = []
        for name, value in sorted((headers or {}).items()):
            pairs.append([name.lower(), value])
        self.script.calls.append(
            {
                "tier": self.tier,
                "url": url,
                "headers": pairs,
                "payload": cast(Json, copy.deepcopy(options.get("json"))),
                "timeout": self.effective_timeout(options.get("timeout")),
            }
        )

    async def post(self, url: str, **options: object) -> httpx.Response:
        self.record(url, options)
        reply = self.script.next_reply()
        self.script.push_during_send(reply)
        request = httpx.Request("POST", url)
        if reply.get("fail") is True:
            raise httpx.ConnectError("the scripted transport failed", request=request)
        return httpx.Response(
            status_code=integer_value(reply.get("status", 0), "reply.status"),
            text=string_value(reply.get("body", ""), "reply.body"),
            request=request,
        )


class ClientFactory:
    def __init__(self, script: Script) -> None:
        self.logging = ScriptedHandler(script, "logging")
        self.moderation = ScriptedHandler(script, "moderation")

    def __call__(
        self,
        llm_provider: str,  # noqa: ARG002 - the patched factory keeps the litellm keyword name
        params: dict[str, object] | None = None,
    ) -> AsyncHTTPHandler:
        if params is not None:
            self.moderation.client_timeout = cast(httpx.Timeout, params.get("timeout"))
            return cast(AsyncHTTPHandler, self.moderation)
        return cast(AsyncHTTPHandler, self.logging)


class FakeTime:
    def __init__(self, values: Sequence[float]) -> None:
        self.values = list(values)
        self.index = 0
        self.last = 0.0

    def time(self) -> float:
        if self.index >= len(self.values):
            return self.last
        value = self.values[self.index]
        self.index += 1
        self.last = value
        return value


class WarningRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.count = 0

    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno == logging.WARNING:
            self.count += 1


def disable_periodic_flush(_logger: RubrikLogger) -> None:
    pass


def classify(error: BaseException) -> str:
    if isinstance(error, httpx.HTTPStatusError):
        return "http_status"
    if isinstance(error, TypeError):
        return "non_object_response"
    if isinstance(error, json.JSONDecodeError):
        return "invalid_body"
    return "request_failed"


async def failure_code(operation: Awaitable[object]) -> Json:
    try:
        await operation
    except (TypeError, ValueError, httpx.HTTPError) as error:
        return classify(error)
    return None


def result(logger: RubrikLogger, error: Json) -> Json:
    return {"queue_length": len(cast("list[Json]", logger.log_queue)), "error": error}


async def perform(logger: RubrikLogger, action: Fields) -> Json:
    step = action.get("do")
    if step == "append":
        await logger._append_and_maybe_flush(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast("dict[str, object]", action["payload"])
        )
        return result(logger, None)
    if step == "append_sequence":
        count = integer_value(action["count"], "append_sequence.count")
        for index in range(count):
            await logger._append_and_maybe_flush({"index": index})  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return result(logger, None)
    if step == "flush":
        await logger.flush_queue()
        return result(logger, None)
    if step == "send_batch":
        return result(logger, await failure_code(logger.async_send_batch()))
    if step == "post_response":
        return result(
            logger,
            await failure_code(
                logger._post_to_response_moderation_endpoint(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    cast(Fields, action["response"]),
                    cast(Fields, action["request"]),
                )
            ),
        )
    if step == "post_prompt":
        return result(
            logger,
            await failure_code(
                logger._post_to_prompt_moderation_endpoint(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    cast(Fields, action["payload"])
                )
            ),
        )
    raise InputError(f"unknown action: {step}")


async def perform_all(logger: RubrikLogger, actions: Sequence[Json]) -> list[Json]:
    return [await perform(logger, object_value(action, "action")) for action in actions]


def logger_options(init: Fields) -> dict[str, object]:
    options: dict[str, object] = {}
    for name in ("api_key", "api_base", "guardrail_name"):
        value = init.get(name)
        if value is not None:
            options[name] = string_value(value, f"init.{name}")
    event_hook = init.get("event_hook")
    if event_hook is not None:
        options["event_hook"] = GuardrailEventHooks(string_value(event_hook, "init.event_hook"))
    default_on = init.get("default_on")
    if default_on is not None:
        options["default_on"] = boolean_value(default_on, "init.default_on")
    timeout = init.get("timeout_seconds")
    if timeout is not None:
        options["timeout"] = number_value(timeout, "init.timeout_seconds")
    return options


def settings_json(logger: RubrikLogger, factory: ClientFactory) -> Fields:
    client_timeout = factory.moderation.client_timeout
    if client_timeout is None:
        raise InputError("the moderation client timeout is unknown")
    override = cast("float | None", logger.timeout)
    if override is None:
        moderation_timeout: Fields = {
            "total": client_timeout.read,
            "connect": client_timeout.connect,
        }
    else:
        moderation_timeout = {"total": override, "connect": override}
    logging_timeout = _default_cached_client_timeout()
    return {
        "guardrail_name": cast(str, logger.guardrail_name),
        "event_hook": cast(GuardrailEventHooks, logger.event_hook).value,
        "default_on": logger.default_on,
        "sampling_rate": logger.sampling_rate,
        "batch_size": logger.batch_size,
        "max_queue_size": logger.max_queue_size,
        "streaming_end_of_stream_only": logger.streaming_end_of_stream_only,
        "streaming_buffer_until_moderated": logger.streaming_buffer_until_moderated,
        "authenticated": logger.key is not None,
        "response_moderation_endpoint": logger.response_moderation_endpoint,
        "prompt_moderation_endpoint": logger.prompt_moderation_endpoint,
        "logging_endpoint": logger.logging_endpoint,
        "moderation_timeout": moderation_timeout,
        "logging_timeout": {"total": logging_timeout.read, "connect": logging_timeout.connect},
    }


def report(
    logger: RubrikLogger,
    script: Script,
    factory: ClientFactory,
    recorder: WarningRecorder,
    results: list[Json],
) -> Json:
    queue = cast("list[Json]", logger.log_queue)
    reporting: Fields = {
        "settings": settings_json(logger, factory),
        "results": results,
        "calls": cast(Json, script.calls),
        "queue": {"length": len(queue), "head": queue[:3], "tail": queue[-3:]},
        "dropped_since_warning": logger._dropped_since_warning,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "last_drop_warning_time": logger._last_drop_warning_time,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "last_flush_time": logger.last_flush_time,
        "warning_count": recorder.count,
    }
    return reporting


def run(input_value: Fields) -> Json:
    environment = object_value(input_value.get("environment", {}), "environment")
    environment_values = {
        name: string_value(value, f"environment.{name}") for name, value in environment.items()
    }
    init = object_value(input_value.get("init", {}), "init")
    script = Script(list_value(input_value.get("replies", []), "replies"))
    clock = list_value(input_value.get("clock", []), "clock")
    clock_values = [number_value(value, "clock entry") for value in clock]
    actions = list_value(input_value.get("actions", []), "actions")
    factory = ClientFactory(script)
    fake_time = FakeTime(clock_values)
    recorder = WarningRecorder()
    verbose_logger.addHandler(recorder)
    try:
        with (
            patch.dict(os.environ),
            patch("litellm.integrations.rubrik.get_async_httpx_client", factory),
            patch("litellm.integrations.rubrik.time", fake_time),
            patch("litellm.integrations.custom_batch_logger.time", fake_time),
            patch.object(RubrikLogger, "_start_periodic_flush_task", disable_periodic_flush),
        ):
            for name in [name for name in os.environ if name.startswith("RUBRIK_")]:
                del os.environ[name]
            os.environ.update(environment_values)
            logger = RubrikLogger(
                **logger_options(init)  # pyright: ignore[reportArgumentType]
            )
            script.logger = logger
            results = asyncio.run(perform_all(logger, actions))
            return report(logger, script, factory, recorder, results)
    finally:
        verbose_logger.removeHandler(recorder)
