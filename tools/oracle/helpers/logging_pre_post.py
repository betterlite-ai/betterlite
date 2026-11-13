from __future__ import annotations

import datetime as dt
import logging
import uuid
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm import _logging
from litellm._logging import verbose_logger
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.litellm_logging import Logging, RawRequestCaptured

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

START_TIME = dt.datetime.fromisoformat("2026-01-01T00:00:00+00:00")
SENTRY_NOTE = "LiteLLM.Logging: is sentry capture exception initialized"


class _Capture(logging.Handler):
    def __init__(self, events: list[Json]) -> None:
        super().__init__(level=logging.DEBUG)
        self.events = events

    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(SENTRY_NOTE):
            return
        self.events.append({"kind": "log", "level": record.levelname.lower(), "message": message})


class _Recorder(CustomLogger):
    def __init__(self, events: list[Json], *, failing: bool) -> None:
        super().__init__()  # pyright: ignore[reportUnknownMemberType]
        self.events = events
        self.failing = failing

    def log_pre_api_call(self, model: str, messages: Json, kwargs: dict[str, Json]) -> None:
        self.events.append(
            {
                "kind": "logger_pre",
                "model": model,
                "messages": messages,
                "log_event_type": kwargs.get("log_event_type"),
            }
        )
        if self.failing:
            raise RuntimeError("callback exploded")

    def log_post_api_call(
        self,
        kwargs: dict[str, Json],
        response_obj: Json,
        start_time: dt.datetime,
        end_time: Json,
    ) -> None:
        del response_obj, end_time
        self.events.append(
            {
                "kind": "logger_post",
                "log_event_type": kwargs.get("log_event_type"),
                "start_time": start_time.isoformat(),
            }
        )
        if self.failing:
            raise RuntimeError("callback exploded")


class _LoggerFn:
    def __init__(self, events: list[Json], *, failing: bool) -> None:
        self.events = events
        self.failing = failing

    def __call__(self, details: dict[str, Json]) -> None:
        self.events.append(
            {
                "kind": "logger_fn",
                "log_event_type": details.get("log_event_type"),
                "input": details.get("input"),
            }
        )
        if self.failing:
            raise RuntimeError("logger_fn exploded")


def run(value: dict[str, Json]) -> Json:
    events: list[Json] = []
    capture = _Capture(events)
    previous_level = verbose_logger.level
    previous_handlers = verbose_logger.handlers
    previous_global = (litellm.turn_off_message_logging, litellm.log_raw_request_response)
    previous_inputs = cast("list[object]", litellm.input_callback)  # pyright: ignore[reportUnknownMemberType]
    verbose_logger.handlers = [capture]
    verbose_logger.setLevel(logging.DEBUG)
    litellm.turn_off_message_logging = value.get("turn_off_message_logging") is True
    litellm.log_raw_request_response = False
    litellm.input_callback = []
    try:
        with (
            patch.object(_logging, "_ENABLE_SECRET_REDACTION", new=False),
            patch.object(
                litellm_logging, "_is_debugging_on", return_value=value.get("verbose") is True
            ),
        ):
            logging_obj = _build(value, events)
            events.clear()
            error = _call(logging_obj, value)
        return _output(logging_obj, events, error)
    finally:
        verbose_logger.handlers = previous_handlers
        verbose_logger.setLevel(previous_level)
        litellm.turn_off_message_logging, litellm.log_raw_request_response = previous_global
        litellm.input_callback = cast("list[str]", previous_inputs)


def _build(value: dict[str, Json], events: list[Json]) -> Logging:
    params = _object(value.get("litellm_params"))
    with (
        patch.object(litellm_logging, "get_litellm_params", return_value=params),
        patch.object(litellm_logging, "scrub_sensitive_keys_in_metadata", _identity),
        patch.object(
            litellm_logging, "_initialize_standard_callback_dynamic_params", return_value={}
        ),
        patch.object(litellm_logging, "get_trusted_callback_params", return_value=()),
        patch.object(Logging, "process_dynamic_callbacks", return_value=None),
        patch.object(Logging, "initialize_standard_built_in_tools_params", return_value={}),
        patch.object(
            litellm_logging.uuid,
            "uuid4",
            return_value=uuid.UUID("00000000-0000-4000-8000-000000000000"),
        ),
    ):
        logging_obj = Logging(
            model="gpt-4o",
            messages=value.get("messages"),
            stream=None,
            call_type=_text(value.get("call_type"), "completion"),
            start_time=START_TIME,
            litellm_call_id="call-1",
            function_id="fn-1",
            litellm_trace_id="trace-1",
            kwargs={},
            log_raw_request_response=value.get("log_raw_request_response") is True,
            raw_request_only=value.get("raw_request_only") is True,
        )
    logging_obj.litellm_request_debug = value.get("request_debug") is True
    logging_obj.dynamic_input_callbacks = [
        _Recorder(events, failing=value.get("failing_callback") is True)
    ]
    logging_obj.logger_fn = _LoggerFn(events, failing=value.get("failing_logger_fn") is True)
    return logging_obj


def _call(logging_obj: Logging, value: dict[str, Json]) -> Json:
    args = _object(value.get("additional_args"))
    try:
        if value.get("action") == "post":
            logging_obj.post_call(  # pyright: ignore[reportUnknownMemberType]
                original_response=value.get("original_response"),
                input=value.get("input"),
                api_key=None,
                additional_args=args,
            )
        else:
            logging_obj.pre_call(  # pyright: ignore[reportUnknownMemberType]
                input=value.get("input"),
                api_key=None,
                model=cast("str | None", value.get("model")),
                additional_args=args,
            )
    except RawRequestCaptured as error:
        return {"class": "RawRequestCaptured", "message": str(error)}
    return None


def _output(logging_obj: Logging, events: list[Json], error: Json) -> Json:
    details = logging_obj.model_call_details
    params = cast("dict[str, Json]", details["litellm_params"])
    return {
        "events": events,
        "details": {
            "log_event_type": details.get("log_event_type"),
            "input": details.get("input"),
            "model": details.get("model"),
            "original_response": details.get("original_response"),
            "api_call_start_time_set": details.get("api_call_start_time") is not None,
            "first_api_call_start_time_set": details.get("first_api_call_start_time") is not None,
            "raw_request_typed_dict": details.get("raw_request_typed_dict"),
        },
        "litellm_params": {
            "api_base": params.get("api_base"),
            "metadata": params.get("metadata"),
        },
        "error": error,
    }


def _identity(params: dict[str, Json]) -> dict[str, Json]:
    return params


def _object(value: Json) -> dict[str, Json]:
    return cast("dict[str, Json]", value) if isinstance(value, dict) else {}


def _text(value: Json, default: str) -> str:
    return value if isinstance(value, str) else default
