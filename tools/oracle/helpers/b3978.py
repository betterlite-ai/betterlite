import asyncio
import json
import logging
import os
import types
from collections.abc import Sequence
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm._logging import verbose_logger
from litellm.exceptions import ModifyResponseException
from litellm.integrations.rubrik import RubrikLogger
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import GenericGuardrailAPIInputs

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
ObjectFields: TypeAlias = dict[str, object]
GuardrailInputType: TypeAlias = Literal["request", "response"]


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


def recorded_json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value, default=dict)))


class Script:
    def __init__(self, replies: Sequence[Json]) -> None:
        self.replies = list(replies)
        self.calls: list[Fields] = []

    async def post(self, url: str, **options: object) -> httpx.Response:
        self.calls.append({"url": url, "payload": recorded_json(options.get("json"))})
        if not self.replies:
            raise InputError("the scripted transport has no reply")
        reply = object_value(self.replies.pop(0), "reply")
        request = httpx.Request("POST", url)
        if reply.get("fail") is True:
            raise httpx.ConnectError("the scripted transport failed", request=request)
        return httpx.Response(
            status_code=integer_value(reply.get("status", 200), "reply.status"),
            text=string_value(reply.get("body", ""), "reply.body"),
            request=request,
        )


class ClientFactory:
    def __init__(self, script: Script) -> None:
        self.script = script

    def __call__(
        self,
        llm_provider: str,  # noqa: ARG002 - the patched factory keeps the litellm keyword name
        params: ObjectFields | None = None,  # noqa: ARG002 - the logging client passes no params
    ) -> AsyncHTTPHandler:
        return cast(AsyncHTTPHandler, self.script)


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


class LogRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.warnings: list[str] = []
        self.critical = 0
        self.errors = 0

    def emit(self, record: logging.LogRecord) -> None:
        if not record.pathname.endswith("litellm/integrations/rubrik.py"):
            return
        if record.levelno >= logging.CRITICAL:
            self.critical += 1
        elif record.levelno >= logging.ERROR:
            self.errors += 1
        elif record.levelno >= logging.WARNING:
            self.warnings.append(record.getMessage())

    def report(self) -> Fields:
        return {
            "warnings": cast(Json, self.warnings),
            "critical": self.critical,
            "errors": self.errors,
        }


def response_object(value: Json) -> object:
    if value is None:
        return None
    fields = object_value(value, "request_data.response")
    if "model" not in fields:
        return types.SimpleNamespace()
    model = fields["model"]
    return types.SimpleNamespace(
        model=None if model is None else string_value(model, "request_data.response.model")
    )


def request_values(value: Json) -> ObjectFields:
    if value is None:
        return {}
    fields = object_value(value, "request_data")
    converted: ObjectFields = {}
    for name, entry in fields.items():
        converted[name] = response_object(entry) if name == "response" else entry
    return converted


def logging_values(value: Json) -> LiteLLMLoggingObj | None:
    if value is None:
        return None
    fields = object_value(value, "logging_obj")
    details = fields.get("model_call_details")
    plain = {} if details is None else dict(object_value(details, "logging_obj.model_call_details"))
    return cast(LiteLLMLoggingObj, types.SimpleNamespace(model_call_details=plain))


def inputs_values(value: Json) -> Fields:
    fields = object_value(value, "inputs")
    texts = [
        string_value(text, "inputs.texts")
        for text in list_value(fields.get("texts", []), "inputs.texts")
    ]
    model = fields.get("model")
    return {
        "texts": cast(Json, texts),
        "tool_calls": fields.get("tool_calls", []),
        "structured_messages": fields.get("structured_messages", []),
        "model": None if model is None else string_value(model, "inputs.model"),
        "tools": fields.get("tools"),
    }


def inputs_view(inputs: Fields) -> Fields:
    return {
        "model": inputs.get("model"),
        "texts": inputs.get("texts", []),
        "tool_calls": inputs.get("tool_calls", []),
        "structured_messages": inputs.get("structured_messages", []),
        "tools": inputs.get("tools"),
    }


def stash_status(request_data: ObjectFields, logging_obj: object) -> str:
    if "_rubrik_logging_obj" not in request_data:
        return "missing"
    stashed = request_data.get("_rubrik_logging_obj")
    if stashed is None:
        return "null"
    return "same" if stashed is logging_obj else "other"


def blocked_flag(logging_obj: LiteLLMLoggingObj | None) -> bool:
    if logging_obj is None:
        return False
    details = cast(ObjectFields, logging_obj.model_call_details)
    return details.get("_rubrik_blocked") is True


async def perform(logger: RubrikLogger, input_value: Fields) -> Fields:
    request_data = request_values(input_value.get("request_data"))
    logging_obj = logging_values(input_value.get("logging_obj"))
    inputs = inputs_values(input_value.get("inputs", {}))
    input_type = string_value(input_value.get("input_type"), "input_type")
    try:
        returned = await logger.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
            cast(GenericGuardrailAPIInputs, inputs),
            request_data,
            cast(GuardrailInputType, input_type),
            logging_obj,
        )
    except ModifyResponseException as error:
        outcome: Json = "blocked"
        reported_inputs: Json = None
        block: Json = {
            "message": error.message,
            "model": error.model,
            "guardrail_name": error.guardrail_name,
        }
    else:
        outcome = "returned"
        reported_inputs = inputs_view(cast(Fields, returned))
        block = None
    return {
        "outcome": outcome,
        "inputs": reported_inputs,
        "block": block,
        "stash": stash_status(request_data, logging_obj),
        "blocked": blocked_flag(logging_obj),
    }


def logger_options(init: Fields) -> ObjectFields:
    options: ObjectFields = {}
    for name in ("api_key", "api_base", "guardrail_name"):
        value = init.get(name)
        if value is not None:
            options[name] = string_value(value, f"init.{name}")
    return options


def disable_periodic_flush(_logger: RubrikLogger) -> None:
    pass


def run(input_value: Fields) -> Json:
    environment = object_value(input_value.get("environment", {}), "environment")
    environment_values = {
        name: string_value(value, f"environment.{name}") for name, value in environment.items()
    }
    init = object_value(input_value.get("init", {}), "init")
    script = Script(list_value(input_value.get("replies", []), "replies"))
    clock_values = list_value(input_value.get("clock", []), "clock")
    clock = [number_value(value, "clock entry") for value in clock_values]
    factory = ClientFactory(script)
    fake_time = FakeTime(clock)
    recorder = LogRecorder()
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
            result = asyncio.run(perform(logger, input_value))
            return {"result": result, "calls": cast(Json, script.calls), "logs": recorder.report()}
    finally:
        verbose_logger.removeHandler(recorder)
