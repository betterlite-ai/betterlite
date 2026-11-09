import asyncio
import copy
import logging
import os
import re
import types
from collections.abc import Sequence
from typing import Literal, Protocol, TypeAlias, cast, runtime_checkable
from unittest.mock import patch

import httpx
from litellm._logging import verbose_logger
from litellm.exceptions import ModifyResponseException
from litellm.integrations.rubrik import (
    RubrikLogger,
    _MalformedToolBlockingResponseError,  # pyright: ignore[reportPrivateUsage]
)
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import GenericGuardrailAPIInputs

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
ObjectFields: TypeAlias = dict[str, object]
GuardrailInputType: TypeAlias = Literal["request", "response"]

_CALL_ID = re.compile(r"litellm_call_id=([^;\s]+)")
_LABEL = re.compile(r"^(.+?) hook failed")


class InputError(ValueError):
    pass


@runtime_checkable
class HasModel(Protocol):
    model: Json


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


class Script:
    def __init__(self, replies: Sequence[Json]) -> None:
        self.replies = list(replies)

    def next_reply(self) -> Fields:
        if not self.replies:
            raise InputError("the scripted transport has no reply")
        return object_value(self.replies.pop(0), "reply")


class ScriptedHandler:
    def __init__(self, script: Script, tier: str) -> None:
        self.script = script
        self.tier = tier
        self.client_timeout: httpx.Timeout | None = None
        self.calls: list[Fields] = []

    def effective_timeout(self, override: object) -> Json:
        if override is not None:
            seconds = number_value(cast(Json, override), "timeout")
            return {"total": seconds, "connect": seconds}
        client_timeout = self.client_timeout
        if client_timeout is None:
            raise InputError("the moderation client timeout is unknown")
        return {"total": client_timeout.read, "connect": client_timeout.connect}

    def record(self, url: str, options: dict[str, object]) -> None:
        headers = cast("dict[str, str] | None", options.get("headers"))
        pairs = cast(Json, {name.lower(): value for name, value in (headers or {}).items()})
        self.calls.append(
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
        self.logging = ScriptedHandler(script, "logging")
        self.moderation = ScriptedHandler(script, "moderation")

    def __call__(
        self,
        llm_provider: str,  # noqa: ARG002 - the patched factory keeps the litellm keyword name
        params: ObjectFields | None = None,
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


class LogRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.warnings: list[str] = []
        self.critical = 0
        self.errors = 0
        self.error_call_ids: list[str] = []
        self.error_labels: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        if not record.pathname.endswith("litellm/integrations/rubrik.py"):
            return
        message = record.getMessage()
        if record.levelno >= logging.CRITICAL:
            self.critical += 1
            return
        if record.levelno >= logging.ERROR:
            self.errors += 1
            call_id = _CALL_ID.search(message)
            if call_id is not None:
                self.error_call_ids.append(call_id.group(1))
            label = _LABEL.search(message)
            if label is not None:
                self.error_labels.append(label.group(1))
            return
        if record.levelno >= logging.WARNING:
            self.warnings.append(message)


def disable_periodic_flush(_logger: RubrikLogger) -> None:
    pass


def response_object(value: Json) -> object:
    if value is None:
        return None
    fields = object_value(value, "request_data.response")
    unknown = set(fields) - {"model"}
    if unknown:
        raise InputError(f"unknown request_data.response field {min(unknown)}")
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
    allowed = {"litellm_call_id", "model", "user", "correlation_key", "tools", "response"}
    converted: ObjectFields = {}
    for name, entry in fields.items():
        if name not in allowed:
            raise InputError(f"unknown request_data field {name}")
        converted[name] = response_object(entry) if name == "response" else entry
    return converted


def logging_values(value: Json) -> LiteLLMLoggingObj | None:
    if value is None:
        return None
    fields = object_value(value, "logging_obj")
    unknown = set(fields) - {"model_call_details"}
    if unknown:
        raise InputError(f"unknown logging_obj field {min(unknown)}")
    details_value = fields.get("model_call_details")
    details: ObjectFields = (
        {}
        if details_value is None
        else dict(object_value(details_value, "logging_obj.model_call_details"))
    )
    logging_obj = types.SimpleNamespace(model_call_details=copy.deepcopy(details))
    return cast(LiteLLMLoggingObj, logging_obj)


def inputs_values(value: Json) -> Fields:
    fields = object_value(value, "inputs")
    allowed = {"texts", "tool_calls", "structured_messages", "model", "tools"}
    unknown = set(fields) - allowed
    if unknown:
        raise InputError(f"unknown inputs field {min(unknown)}")
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


def model_call_details(logging_obj: LiteLLMLoggingObj | None) -> Json:
    if logging_obj is None:
        return None
    details: object = logging_obj.model_call_details
    return cast(Json, details)


def request_data_view(request_data: ObjectFields, logging_obj: object) -> Fields:
    response = request_data.get("response")
    stash_present = "_rubrik_logging_obj" in request_data
    stashed = request_data.get("_rubrik_logging_obj")
    if not stash_present:
        stash = "missing"
    elif stashed is None:
        stash = "null"
    elif stashed is logging_obj:
        stash = "same"
    else:
        stash = "other"
    if isinstance(response, HasModel):
        response_model: Json = response.model
    else:
        response_model = None
    return {
        "litellm_call_id": cast(Json, request_data.get("litellm_call_id")),
        "model": cast(Json, request_data.get("model")),
        "user": cast(Json, request_data.get("user")),
        "correlation_key": cast(Json, request_data.get("correlation_key")),
        "tools_present": "tools" in request_data,
        "tools": cast(Json, request_data.get("tools")),
        "response_present": "response" in request_data and response is not None,
        "response_has_model": isinstance(response, HasModel),
        "response_model": response_model,
        "stash": stash,
    }


def guardrail_name(logger: RubrikLogger) -> str | None:
    return logger.guardrail_name


def blocked_result(
    exception: ModifyResponseException,
    request_data: ObjectFields,
    logging_obj: LiteLLMLoggingObj | None,
) -> Fields:
    exception_data = cast(ObjectFields, exception.request_data)
    return {
        "outcome": "blocked",
        "inputs": None,
        "block": {
            "message": exception.message,
            "model": exception.model,
            "guardrail_name": exception.guardrail_name,
            "request_data": request_data_view(exception_data, logging_obj),
        },
        "request_data": request_data_view(request_data, logging_obj),
        "model_call_details": model_call_details(logging_obj),
    }


def returned_result(
    returned: Fields,
    request_data: ObjectFields,
    logging_obj: LiteLLMLoggingObj | None,
) -> Fields:
    return {
        "outcome": "returned",
        "inputs": inputs_view(returned),
        "block": None,
        "request_data": request_data_view(request_data, logging_obj),
        "model_call_details": model_call_details(logging_obj),
    }


def stashed_result(
    request_data: ObjectFields,
    logging_obj: LiteLLMLoggingObj | None,
) -> Fields:
    return {
        "outcome": "stashed",
        "inputs": None,
        "block": None,
        "request_data": request_data_view(request_data, logging_obj),
        "model_call_details": model_call_details(logging_obj),
    }


def guarded_fault(fault: str, logger: RubrikLogger, request_data: ObjectFields) -> BaseException:
    if fault == "blocked":
        return ModifyResponseException(
            message="synthetic block",
            model="gpt-4o",
            request_data=request_data,
            guardrail_name=guardrail_name(logger),
        )
    if fault == "malformed":
        return _MalformedToolBlockingResponseError(  # pyright: ignore[reportPrivateUsage]
            "Response moderation service returned empty response"
        )
    if fault == "transient":
        return httpx.ConnectError(
            "the scripted transport failed",
            request=httpx.Request("POST", "https://rubrik.test"),
        )
    raise InputError(f"unknown guarded fault: {fault}")


async def perform_guarded(
    logger: RubrikLogger,
    action: Fields,
    request_data: ObjectFields,
    logging_obj: LiteLLMLoggingObj | None,
) -> Fields:
    inputs = inputs_values(action.get("inputs", {}))
    fault = string_value(action.get("fault"), "action.fault")
    loop = asyncio.get_running_loop()
    future = cast(
        "asyncio.Future[GenericGuardrailAPIInputs]", loop.create_future()
    )
    future.set_exception(guarded_fault(fault, logger, request_data))
    try:
        returned = await RubrikLogger._guarded(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            future,
            cast(GenericGuardrailAPIInputs, inputs),
            "Response moderation",
        )
    except ModifyResponseException as error:
        return blocked_result(error, request_data, logging_obj)
    return returned_result(cast(Fields, returned), request_data, logging_obj)


async def perform(
    logger: RubrikLogger,
    action: Fields,
    request_data: ObjectFields,
    logging_obj: LiteLLMLoggingObj | None,
) -> Fields:
    step = action.get("do")
    if step == "stash_block_context":
        RubrikLogger._stash_block_context(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            logging_obj, request_data
        )
        return stashed_result(request_data, logging_obj)
    if step == "guarded":
        return await perform_guarded(logger, action, request_data, logging_obj)
    if step != "apply_guardrail":
        raise InputError(f"unknown action: {step}")
    inputs = inputs_values(action.get("inputs", {}))
    input_type = string_value(action.get("input_type"), "action.input_type")
    try:
        returned = await logger.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
            cast(GenericGuardrailAPIInputs, inputs),
            request_data,
            cast(GuardrailInputType, input_type),
            logging_obj,
        )
    except ModifyResponseException as error:
        return blocked_result(error, request_data, logging_obj)
    return returned_result(cast(Fields, returned), request_data, logging_obj)


async def perform_all(
    logger: RubrikLogger,
    actions: Sequence[Json],
    request_data: ObjectFields,
    logging_obj: LiteLLMLoggingObj | None,
) -> list[Json]:
    return [
        await perform(logger, object_value(action, "action"), request_data, logging_obj)
        for action in actions
    ]


def logger_options(init: Fields) -> ObjectFields:
    options: ObjectFields = {}
    for name in ("api_key", "api_base", "guardrail_name"):
        value = init.get(name)
        if value is not None:
            options[name] = string_value(value, f"init.{name}")
    timeout = init.get("timeout_seconds")
    if timeout is not None:
        options["timeout"] = number_value(timeout, "init.timeout_seconds")
    return options


def report(
    factory: ClientFactory,
    recorder: LogRecorder,
    results: list[Json],
) -> Fields:
    return {
        "results": results,
        "calls": cast(Json, factory.moderation.calls),
        "logs": {
            "warnings": cast(Json, recorder.warnings),
            "critical": recorder.critical,
            "errors": recorder.errors,
            "error_call_ids": cast(Json, recorder.error_call_ids),
            "error_labels": cast(Json, recorder.error_labels),
        },
    }


def run(input_value: Fields) -> Json:
    environment = object_value(input_value.get("environment", {}), "environment")
    environment_values = {
        name: string_value(value, f"environment.{name}") for name, value in environment.items()
    }
    init = object_value(input_value.get("init", {}), "init")
    script = Script(list_value(input_value.get("replies", []), "replies"))
    clock_values = list_value(input_value.get("clock", []), "clock")
    clock = [number_value(value, "clock entry") for value in clock_values]
    actions = list_value(input_value.get("actions", []), "actions")
    request_data = request_values(input_value.get("request_data"))
    logging_obj = logging_values(input_value.get("logging_obj"))
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
            results = asyncio.run(perform_all(logger, actions, request_data, logging_obj))
            return report(factory, recorder, results)
    finally:
        verbose_logger.removeHandler(recorder)
