from __future__ import annotations

import calendar
import datetime as dt
import uuid
from collections.abc import Callable
from typing import TYPE_CHECKING, Literal, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm._logging import session_id_var, trace_id_var
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.litellm_logging import Logging

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
EventType: TypeAlias = Literal["async_success", "sync_success", "async_failure", "sync_failure"]
CallbackValue: TypeAlias = str | Callable[[], None] | CustomLogger
Operation: TypeAlias = Callable[[Logging, "dict[str, Json]"], Json]

DEFAULT_GENERATED_TRACE_ID = "00000000-0000-4000-8000-000000000000"
ProxySpendLogger = cast("type[CustomLogger]", type("_PROXY_SpendLogger", (CustomLogger,), {}))


def run(input_value: dict[str, Json]) -> Json:
    instances = input_value.get("instances")
    if not isinstance(instances, list):
        raise ValueError("instances must be a list")
    return {"instances": [_instance(cast("dict[str, Json]", value)) for value in instances]}


def _instance(value: dict[str, Json]) -> Json:
    init = _object(value.get("init"))
    correlation = _object(value.get("correlation"))
    settings = _object(correlation.get("settings"))
    context = _object(correlation.get("context"))
    operations = value.get("operations")
    if not isinstance(operations, list):
        raise ValueError("operations must be a list")
    trace_id_var.set(_text_or_empty(context.get("trace_id")))
    session_id_var.set(_text_or_empty(context.get("session_id")))
    previous_flag = litellm.request_correlation_in_logs
    previous_disable = litellm.global_disable_no_log_param
    litellm.request_correlation_in_logs = settings.get("request_correlation_in_logs") is True
    litellm.global_disable_no_log_param = (
        _object(init.get("callback_policy")).get("global_disable_no_log_param") is True
    )
    generated = correlation.get("generated_trace_id")
    generated_id = DEFAULT_GENERATED_TRACE_ID if generated is None else _string(generated)
    try:
        logging_obj = _build(init, settings, generated_id)
        snapshot = _snapshot(logging_obj)
        results = [
            _operation(logging_obj, cast("dict[str, Json]", operation))
            for operation in operations
        ]
        return {"init": snapshot, "results": results}
    finally:
        litellm.request_correlation_in_logs = previous_flag
        litellm.global_disable_no_log_param = previous_disable
        trace_id_var.set("")
        session_id_var.set("")


def _build(init: dict[str, Json], settings: dict[str, Json], generated: str) -> Logging:
    dynamic = _object(init.get("dynamic_callbacks"))
    flags = _object(init.get("request_logging"))
    with (
        patch.object(
            litellm_logging,
            "get_litellm_params",
            return_value=_object(init.get("litellm_params")),
        ),
        patch.object(litellm_logging, "scrub_sensitive_keys_in_metadata", _identity),
        patch.object(
            litellm_logging,
            "_initialize_standard_callback_dynamic_params",
            return_value=_object(init.get("standard_callback_dynamic_params")),
        ),
        patch.object(
            litellm_logging,
            "get_trusted_callback_params",
            return_value=_trusted(init.get("trusted_callback_vars")),
        ),
        patch.object(Logging, "process_dynamic_callbacks", return_value=None),
        patch.object(
            Logging,
            "initialize_standard_built_in_tools_params",
            return_value=_object(init.get("standard_built_in_tools_params")),
        ),
        patch.object(litellm_logging.uuid, "uuid4", return_value=uuid.UUID(generated)),
    ):
        return Logging(
            model=_string(init.get("model")),
            messages=init.get("messages"),
            stream=init.get("stream"),
            call_type=_string(init.get("call_type")),
            start_time=dt.datetime.fromisoformat(_string(init.get("start_time"))),
            litellm_call_id=_string(init.get("litellm_call_id")),
            function_id=_string(init.get("function_id")),
            litellm_trace_id=cast("str | None", init.get("litellm_trace_id")),
            dynamic_input_callbacks=_callbacks(dynamic.get("input")),
            dynamic_success_callbacks=_callbacks(dynamic.get("success")),
            dynamic_async_success_callbacks=_callbacks(dynamic.get("async_success")),
            dynamic_failure_callbacks=_callbacks(dynamic.get("failure")),
            dynamic_async_failure_callbacks=_callbacks(dynamic.get("async_failure")),
            kwargs=_object(init.get("kwargs")),
            log_raw_request_response=flags.get("log_raw_request_response") is True,
            supports_correlation_logging=settings.get("supports_correlation_logging") is True,
            raw_request_only=flags.get("raw_request_only") is True,
        )


def _identity(params: dict[str, Json]) -> dict[str, Json]:
    return params


def _operation(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    name = operation.get("op")
    handler = _OPERATIONS.get(cast(str, name))
    if handler is None:
        raise ValueError(f"unknown logging_state operation {name}")
    return handler(logging_obj, operation)


def _set_timing(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    logging_obj.set_response_timing_metrics(_metrics(operation.get("metrics")))
    return {"response_timing_metrics": dict(logging_obj.response_timing_metrics)}


def _update_environment(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    update = _object(operation.get("update"))
    logging_obj.update_environment_variables(  # pyright: ignore[reportUnknownMemberType]
        litellm_params=_object(operation.get("litellm_params")),
        optional_params=_object(update.get("optional_params")),
        model=_optional_string(update.get("model")),
        user=_optional_string(update.get("user")),
        **_object(update.get("additional_params")),
    )
    return _snapshot(logging_obj)


def _update_kwargs(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    update = _object(operation.get("update"))
    logging_obj.update_from_kwargs(  # pyright: ignore[reportUnknownMemberType]
        kwargs=_object(operation.get("kwargs")),
        litellm_params=cast("dict[str, Json] | None", operation.get("litellm_params")),
        optional_params=_object(update.get("optional_params")),
        model=_optional_string(update.get("model")),
        user=_optional_string(update.get("user")),
        **_object(update.get("additional_params")),
    )
    return _snapshot(logging_obj)


def _update_messages(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    logging_obj.update_messages(cast("list[AllMessageValues]", operation.get("messages")))
    return _snapshot(logging_obj)


def _is_sync(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    del logging_obj
    return Logging._is_sync_litellm_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        _object(operation.get("litellm_params"))
    )


def _should_run(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    return logging_obj.should_run_logging(cast(EventType, operation.get("event")))


def _has_run(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    logging_obj.has_run_logging(cast(EventType, operation.get("event")))
    return None


def _claim(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    del operation
    return logging_obj.claim_async_success_log()


def _should_run_callback(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    return logging_obj.should_run_callback(  # pyright: ignore[reportUnknownMemberType]
        _callback(operation.get("callback")),
        _object(operation.get("litellm_params")),
        "sync_success",
    )


def _completion_start(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    logging_obj._update_completion_start_time(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        dt.datetime.fromisoformat(_string(operation.get("completion_start_time")))
    )
    return _snapshot(logging_obj)


def _restore(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    del operation
    logging_obj._restore_correlation_context()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return _context()


def _restore_guarded(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    del operation
    logging_obj._restore_correlation_context_if_unclaimed()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return _context()


def _set_correlation(logging_obj: Logging, operation: dict[str, Json]) -> Json:
    del logging_obj
    trace_id_var.set(_text_or_empty(operation.get("trace_id")))
    session_id_var.set(_text_or_empty(operation.get("session_id")))
    return _context()


_OPERATIONS: dict[str, Operation] = {
    "set_response_timing_metrics": _set_timing,
    "update_environment_variables": _update_environment,
    "update_from_kwargs": _update_kwargs,
    "update_messages": _update_messages,
    "is_sync_litellm_request": _is_sync,
    "should_run_logging": _should_run,
    "has_run_logging": _has_run,
    "claim_async_success_log": _claim,
    "should_run_callback": _should_run_callback,
    "update_completion_start_time": _completion_start,
    "restore_correlation": _restore,
    "restore_correlation_guarded": _restore_guarded,
    "set_correlation": _set_correlation,
}


def _snapshot(logging_obj: Logging) -> Json:
    completion = logging_obj.completion_start_time
    return cast(
        Json,
        {
            "model": logging_obj.model,
            "messages": logging_obj.messages,
            "stream": logging_obj.stream,
            "call_type": logging_obj.call_type,
            "start_time_micros": _micros(logging_obj.start_time),
            "litellm_call_id": logging_obj.litellm_call_id,
            "litellm_trace_id": logging_obj.litellm_trace_id,
            "litellm_session_id": logging_obj.litellm_session_id,
            "function_id": logging_obj.function_id,
            "litellm_params": logging_obj.litellm_params,
            "optional_params": getattr(logging_obj, "optional_params", {}),
            "user": getattr(logging_obj, "user", None),
            "litellm_request_debug": logging_obj.litellm_request_debug,
            "stream_options": getattr(logging_obj, "stream_options", None),
            "custom_pricing": logging_obj.custom_pricing,
            "zero_cost_warned": logging_obj.zero_cost_warned,
            "custom_llm_provider": getattr(logging_obj, "custom_llm_provider", None),
            "completion_start_time_micros": None if completion is None else _micros(completion),
            "response_timing_metrics": dict(logging_obj.response_timing_metrics),
            "model_call_details": _plain(logging_obj.model_call_details),
            "callback_dynamic_params": logging_obj.standard_callback_dynamic_params,
            "trusted_callback_vars": [
                list(pair)
                for pair in logging_obj._trusted_callback_vars  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            ],
            "standard_built_in_tools_params": logging_obj.standard_built_in_tools_params,
            "request_logging": {
                "log_raw_request_response": logging_obj.log_raw_request_response,
                "raw_request_only": logging_obj.raw_request_only,
            },
            "dynamic_callbacks": {
                "input": logging_obj.dynamic_input_callbacks,
                "success": logging_obj.dynamic_success_callbacks,
                "async_success": logging_obj.dynamic_async_success_callbacks,
                "failure": logging_obj.dynamic_failure_callbacks,
                "async_failure": logging_obj.dynamic_async_failure_callbacks,
            },
            "truncated_messages_for_logging": logging_obj.truncated_messages_for_logging,
            "cost_breakdown": logging_obj.cost_breakdown,
            "correlation": {
                "pre_call": {
                    "trace_id": logging_obj._pre_call_trace_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    "session_id": logging_obj._pre_call_session_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                },
                "own": {
                    "trace_id": logging_obj._own_trace_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    "session_id": logging_obj._own_session_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                },
            },
            "context": _context(),
            "async_success_scheduled": logging_obj._async_success_scheduled,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        },
    )


def _context() -> Json:
    return {"trace_id": trace_id_var.get(), "session_id": session_id_var.get()}


def _micros(moment: dt.datetime) -> int:
    return calendar.timegm(moment.utctimetuple()) * 1_000_000 + moment.microsecond


def _plain(value: object) -> Json:
    if isinstance(value, dt.datetime):
        return value.isoformat()
    if isinstance(value, dict):
        return {
            str(name): _plain(entry)
            for name, entry in cast("dict[object, object]", value).items()
        }
    if isinstance(value, (list, tuple)):
        return [_plain(entry) for entry in cast("list[object]", value)]
    return cast(Json, value)


def _callback(value: Json) -> str | CustomLogger:
    return ProxySpendLogger() if value == "proxy_internal" else "langfuse"


def _metrics(value: Json) -> dict[str, float]:
    return {name: cast(float, entry) for name, entry in _object(value).items()}


def _callbacks(value: Json) -> list[CallbackValue] | None:
    if value is None:
        return None
    return cast("list[CallbackValue]", value)


def _trusted(value: Json) -> tuple[tuple[str, str], ...]:
    pairs = cast("list[list[str]]", value) if value is not None else []
    return tuple((pair[0], pair[1]) for pair in pairs)


def _object(value: Json) -> dict[str, Json]:
    return cast("dict[str, Json]", value) if isinstance(value, dict) else {}


def _string(value: Json) -> str:
    return cast(str, value)


def _text_or_empty(value: Json) -> str:
    return value if isinstance(value, str) else ""


def _optional_string(value: Json) -> str | None:
    return cast("str | None", value)
