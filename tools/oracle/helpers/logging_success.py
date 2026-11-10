from __future__ import annotations

import asyncio
import datetime as dt
from collections.abc import Callable, Generator
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from typing import Final, TypeAlias, cast
from unittest.mock import AsyncMock, Mock, patch

import litellm
from litellm._logging import session_id_var, trace_id_var
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import (
    Choices,
    EmbeddingResponse,
    Message,
    ModelResponse,
    ModelResponseStream,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Loggers: TypeAlias = list[CustomLogger]
Records: TypeAlias = list[list[Fields]]
Operation: TypeAlias = Callable[[list[Fields]], Json]


class InputError(ValueError):
    pass


def _object(value: Json, name: str) -> Fields:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return cast("Fields", value)


def _objects(value: Json, name: str) -> list[Fields]:
    return [_object(item, name) for item in _items(value, name)]


def _items(value: Json, name: str) -> list[Json]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return cast("list[Json]", value)


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def _strings(value: Json, name: str) -> list[str]:
    return [_text(item, name) for item in _items(value, name)]


def _boolean(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise InputError(f"{name} must be a bool")
    return value


def _integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def _optional_bool(value: Json) -> bool | None:
    return value if isinstance(value, bool) else None


def _optional_text(value: Json) -> str | None:
    return value if isinstance(value, str) else None


def _datetime(value: Json, name: str) -> dt.datetime:
    return dt.datetime.fromisoformat(_text(value, name))


def _context_values(value: Json) -> tuple[str, str]:
    fields = _object(value, "context")
    return (
        _optional_text(fields.get("trace_id")) or "",
        _optional_text(fields.get("session_id")) or "",
    )


class _RecordingCounter(CustomLogger):
    def __init__(self) -> None:
        super().__init__()  # pyright: ignore[reportUnknownMemberType]
        self.recorded: list[str] = []

    def increment_callback_logging_failure(self, callback_name: str) -> None:
        self.recorded.append(callback_name)


@dataclass
class _Globals:
    callbacks: list[object]
    success_callback: list[object]
    async_success_callback: list[object]
    failure_callback: list[object]
    async_failure_callback: list[object]
    excluded_fields: object
    max_budget: object
    request_correlation: bool
    turn_off_message_logging: object


def _save_globals() -> _Globals:
    return _Globals(
        callbacks=cast("list[object]", litellm.callbacks),
        success_callback=cast("list[object]", litellm.success_callback),
        async_success_callback=cast(
            "list[object]",
            litellm._async_success_callback,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        ),
        failure_callback=cast("list[object]", litellm.failure_callback),
        async_failure_callback=cast(
            "list[object]",
            litellm._async_failure_callback,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        ),
        excluded_fields=cast(
            "object",
            getattr(litellm, "standard_logging_payload_excluded_fields", None),
        ),
        max_budget=cast("object", litellm.max_budget),
        request_correlation=litellm.request_correlation_in_logs,
        turn_off_message_logging=cast("object", litellm.turn_off_message_logging),
    )


def _restore_globals(saved: _Globals) -> None:
    litellm.callbacks = saved.callbacks
    litellm.success_callback = saved.success_callback
    litellm._async_success_callback = (  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        saved.async_success_callback
    )
    litellm.failure_callback = saved.failure_callback
    litellm._async_failure_callback = (  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        saved.async_failure_callback
    )
    litellm.standard_logging_payload_excluded_fields = cast(
        "list[str] | None", saved.excluded_fields
    )
    litellm.max_budget = cast("float | None", saved.max_budget)
    litellm.request_correlation_in_logs = saved.request_correlation
    litellm.turn_off_message_logging = cast("bool | None", saved.turn_off_message_logging)


def _getter(value: Json) -> Callable[..., Json]:
    def get(*_args: object, **_kwargs: object) -> Json:
        return value

    return get


def _trusted(*_args: object, **_kwargs: object) -> tuple[tuple[str, str], ...]:
    return ()


def _identity_details(params: Fields) -> Fields:
    return params


def _process_details(payload: Json) -> Callable[..., None]:
    def process(
        self: Logging,
        *,
        logging_result: object,
        start_time: object,
        end_time: object,
        build_logging_payload: bool = True,
    ) -> None:
        del logging_result, start_time, end_time, build_logging_payload
        self.model_call_details["standard_logging_object"] = payload

    return process


def _build_payload(payload: Json) -> Callable[..., Json]:
    def build(
        self: Logging,
        init_response_obj: object,
        start_time: object,
        end_time: object,
    ) -> Json:
        del self, init_response_obj, start_time, end_time
        return payload

    return build


def _assemble(complete: Json, *, declines_results: bool) -> Callable[..., object]:
    def assemble(  # noqa: PLR0913
        self: Logging,
        *,
        result: object,
        start_time: object,
        end_time: object,
        is_async: bool,
        streaming_chunks: object,
    ) -> object:
        del start_time, end_time, is_async, streaming_chunks
        if self.stream is not True:  # pyright: ignore[reportUnknownMemberType]
            return None
        if isinstance(result, ModelResponseStream):
            return complete
        return None if declines_results else result

    return assemble


def _no_value(*_args: object, **_kwargs: object) -> None:
    return None


def _stamp(probe: Fields) -> tuple[str, int]:
    stamp = _object(probe.get("stamp"), "stamp") if probe.get("stamp") is not None else {}
    identifier = _optional_text(stamp.get("id")) or "chatcmpl-placeholder"
    created = stamp.get("created")
    seconds = _integer(created, "stamp.created") if created is not None else 1_704_067_200
    return (identifier, seconds)


async def _no_truncate(messages: object) -> object:
    return messages


@contextmanager
def _probe_patches(probe: Fields) -> Generator[None]:
    params = _object(probe.get("litellm_params"), "litellm_params")
    built = probe.get("built_payload")
    complete = probe.get("complete_response")
    dynamic = _object(probe.get("dynamic_params"), "dynamic_params")
    declines_results = _boolean(
        probe.get("assembler_returns_none", False), "assembler_returns_none"
    )
    saved = _save_globals()
    litellm.max_budget = 0
    litellm.turn_off_message_logging = _boolean(
        probe.get("global_redaction", False), "global_redaction"
    )
    litellm.request_correlation_in_logs = _boolean(
        probe.get("request_correlation", False), "request_correlation"
    )
    try:
        with ExitStack() as stack:
            stack.enter_context(
                patch.object(litellm_logging, "get_litellm_params", _getter(params))
            )
            stack.enter_context(
                patch.object(
                    litellm_logging, "scrub_sensitive_keys_in_metadata", _identity_details
                )
            )
            stack.enter_context(
                patch.object(
                    litellm_logging,
                    "_initialize_standard_callback_dynamic_params",
                    _getter(dynamic),
                )
            )
            stack.enter_context(
                patch.object(litellm_logging, "get_trusted_callback_params", _trusted)
            )
            stack.enter_context(patch.object(Logging, "process_dynamic_callbacks", _no_value))
            stack.enter_context(
                patch.object(Logging, "initialize_standard_built_in_tools_params", _getter({}))
            )
            stack.enter_context(
                patch.object(
                    Logging, "_process_hidden_params_and_response_cost", _process_details(built)
                )
            )
            stack.enter_context(
                patch.object(Logging, "_build_standard_logging_payload", _build_payload(built))
            )
            stack.enter_context(
                patch.object(Logging, "_response_cost_calculator", _no_value)
            )
            stack.enter_context(
                patch.object(Logging, "_surface_response_headers_from_result", _no_value)
            )
            stack.enter_context(
                patch.object(
                    Logging, "_merge_hidden_params_from_response_into_metadata", _no_value
                )
            )
            stack.enter_context(
                patch.object(
                    Logging,
                    "_get_assembled_streaming_response",
                    _assemble(complete, declines_results=declines_results),
                )
            )
            stack.enter_context(
                patch.object(litellm_logging, "emit_standard_logging_payload", _no_value)
            )
            stack.enter_context(
                patch.object(litellm_logging, "truncate_base64_in_messages_async", _no_truncate)
            )
            stack.enter_context(
                patch.object(litellm_logging, "flush_post_call_redis_batches", AsyncMock())
            )
            stamp_id, stamp_created = _stamp(probe)
            stack.enter_context(
                patch("litellm.types.utils._generate_id", _getter(stamp_id))
            )
            stack.enter_context(
                patch(
                    "litellm.types.utils.time",
                    Mock(time=Mock(return_value=float(stamp_created))),
                )
            )
            yield
    finally:
        _restore_globals(saved)


def _logger_class(
    name: str,
    *,
    redacts: bool,
    fail: bool,
    records: list[Fields],
) -> type[CustomLogger]:
    def record(event_name: str, kwargs: object) -> None:
        fields = cast("Fields", kwargs) if isinstance(kwargs, dict) else {}
        standard = fields.get("standard_logging_object")
        payload: Fields = cast("Fields", standard) if isinstance(standard, dict) else {}
        records.append(
            {
                "logger": name,
                "event": event_name,
                "stream": fields.get("stream") is True,
                "model": payload.get("model"),
                "messages": payload.get("messages"),
                "response": payload.get("response"),
            }
        )
        if fail:
            raise RuntimeError("the transport is down")

    def log_success_event(
        self: CustomLogger,
        kwargs: object,
        response_obj: object,
        start_time: object,
        end_time: object,
    ) -> None:
        del self, response_obj, start_time, end_time
        record("log_success_event", kwargs)

    def log_stream_event(
        self: CustomLogger,
        kwargs: object,
        response_obj: object,
        start_time: object,
        end_time: object,
    ) -> None:
        del self, response_obj, start_time, end_time
        record("log_stream_event", kwargs)

    async def async_log_success_event(
        self: CustomLogger,
        kwargs: object,
        response_obj: object,
        start_time: object,
        end_time: object,
    ) -> None:
        del self, response_obj, start_time, end_time
        record("async_log_success_event", kwargs)

    async def async_log_stream_event(
        self: CustomLogger,
        kwargs: object,
        response_obj: object,
        start_time: object,
        end_time: object,
    ) -> None:
        del self, response_obj, start_time, end_time
        record("async_log_stream_event", kwargs)

    namespace: dict[str, object] = {
        "log_success_event": log_success_event,
        "log_stream_event": log_stream_event,
        "async_log_success_event": async_log_success_event,
        "async_log_stream_event": async_log_stream_event,
    }
    if redacts:

        def redacts_messages_itself(self: CustomLogger) -> bool:
            del self
            return True

        namespace["redacts_messages_itself"] = redacts_messages_itself
    return cast("type[CustomLogger]", type(name, (CustomLogger,), namespace))


def _loggers(specs: list[Fields]) -> tuple[Loggers, Records]:
    loggers: Loggers = []
    records: Records = []
    for spec in specs:
        name = _text(spec.get("name"), "loggers[].name")
        turn_off = _boolean(spec.get("turn_off_message_logging", False), "turn_off_message_logging")
        redacts = _boolean(spec.get("redacts_messages_itself", False), "redacts_messages_itself")
        message_logging = _boolean(spec.get("message_logging", True), "message_logging")
        fail = _boolean(spec.get("fail", False), "loggers[].fail")
        calls: list[Fields] = []
        logger_class = _logger_class(name, redacts=redacts, fail=fail, records=calls)
        loggers.append(
            logger_class(turn_off_message_logging=turn_off, message_logging=message_logging)
        )
        records.append(calls)
    return loggers, records


@dataclass
class _Run:
    obj: Logging
    records: Records
    counters: list[_RecordingCounter]


def _prepare(probe: Fields, list_mode: str) -> _Run:
    loggers, records = _loggers(_objects(probe.get("loggers"), "loggers"))
    counters = [_RecordingCounter(), _RecordingCounter()]
    litellm.callbacks = cast("list[object]", list(counters))
    litellm.success_callback = cast(
        "list[object]", list(loggers) if list_mode in ("sync", "both") else []
    )
    litellm._async_success_callback = cast(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "list[object]", list(loggers) if list_mode in ("async", "both") else []
    )
    litellm.failure_callback = []
    litellm._async_failure_callback = []  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    litellm.standard_logging_payload_excluded_fields = _strings(
        probe.get("excluded_fields"), "excluded_fields"
    )
    trace_id, session_id = _context_values(probe.get("context"))
    trace_id_var.set(trace_id)
    session_id_var.set(session_id)
    kwargs: Fields = {}
    session = _optional_text(probe.get("session_id"))
    if session is not None:
        kwargs["litellm_session_id"] = session
    obj = Logging(
        model="gpt-4o",
        messages=None,
        stream=_boolean(probe.get("stream", False), "stream"),
        call_type=_text(probe.get("call_type", "completion"), "call_type"),
        start_time=_datetime(probe.get("start_time"), "start_time")
        if probe.get("start_time") is not None
        else dt.datetime.fromisoformat("2024-01-01T00:00:00+00:00"),
        litellm_call_id="call-1",
        function_id="function-1",
        litellm_trace_id=_optional_text(probe.get("trace_id")),
        kwargs=kwargs,
    )
    obj.model_call_details["stream"] = obj.stream  # pyright: ignore[reportUnknownMemberType]
    return _Run(obj=obj, records=records, counters=counters)


def _result(spec: Json) -> object:
    fields = _object(spec, "result")
    kind = _text(fields.get("kind"), "result.kind")
    value = fields.get("value")
    match kind:
        case "chat":
            return ModelResponse(id="chatcmpl-1", choices=[Choices(message=Message(content="hi"))])
        case "embeddings":
            return EmbeddingResponse(model="text-embedding-3-small", data=[])
        case "chunk":
            return ModelResponseStream(id="chatcmpl-chunk")
        case "other":
            return value
        case _:
            raise InputError(f"unknown result kind {kind}")


def _events(records: Records) -> Json:
    return cast("Json", [call for calls in records for call in calls])


def _details(obj: Logging) -> Json:
    details = obj.model_call_details
    return {
        "log_event_type": _optional_text(details.get("log_event_type")),
        "cache_hit": _optional_bool(details.get("cache_hit")),
        "completion_start_time": _isoformat(details.get("completion_start_time")),
        "end_time": _isoformat(details.get("end_time")),
    }


def _isoformat(value: object) -> Json:
    if isinstance(value, dt.datetime):
        return value.isoformat()
    return cast("Json", value) if isinstance(value, str) else None


def _state_value(key: str | None) -> Json:
    return None if key == "null" else key


def _assembled(probes: list[Fields]) -> Json:
    results: list[Json] = []
    for probe in probes:
        with _probe_patches(probe):
            run = _prepare(probe, "sync")
            state = run.obj.model_call_details
            async_key = _optional_text(probe.get("async_key"))
            complete_key = _optional_text(probe.get("complete_key"))
            if async_key is not None:
                state["async_complete_streaming_response"] = _state_value(async_key)
            if complete_key is not None:
                state["complete_streaming_response"] = _state_value(complete_key)
            assembled = run.obj._is_assembled_stream_success(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                _result(probe.get("result"))
            )
            results.append(cast("Json", assembled))
    return {"operation": "assembled", "results": results}


def _recognized(probes: list[Fields]) -> Json:
    results: list[Json] = []
    for probe in probes:
        with _probe_patches(probe):
            run = _prepare(probe, "sync")
            results.append(
                cast(
                    "Json",
                    run.obj._is_recognized_call_type_for_logging(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                        _result(probe.get("result"))
                    ),
                )
            )
    return {"operation": "recognized", "results": results}


def _dispatch(probes: list[Fields]) -> Json:
    outputs: list[Json] = []
    for probe in probes:
        with _probe_patches(probe):
            run = _prepare(probe, "both")
            marker = probe.get("marker") is True
            if marker:
                run.obj.model_call_details["has_dispatched_final_stream_success"] = True
            result = _result(probe.get("result"))
            prefer = _boolean(probe.get("prefer_async_handlers", False), "prefer_async_handlers")
            dispatches = probe.get("dispatches")
            count = dispatches if isinstance(dispatches, int) else 1
            with patch(
                "litellm.litellm_core_utils.thread_pool_executor.executor",
                _InlineExecutor(),
            ):
                for _ in range(count):
                    asyncio.run(
                        run.obj.dispatch_success_handlers(  # pyright: ignore[reportUnknownMemberType]
                            result=result, prefer_async_handlers=prefer
                        )
                    )
            outputs.append(
                {
                    "events": _events(run.records),
                    "marker": run.obj.model_call_details.get(
                        "has_dispatched_final_stream_success"
                    )
                    is True,
                }
            )
    return {"operation": "dispatch", "probes": outputs}


class _InlineExecutor:
    def submit(self, function: Callable[..., object], *args: object, **kwargs: object) -> None:
        function(*args, **kwargs)


def _handler(probes: list[Fields]) -> Json:
    outputs: list[Json] = []
    for probe in probes:
        mode = _text(probe.get("mode"), "mode")
        outputs.append(
            asyncio.run(_async_handler_probe(probe))
            if mode == "async"
            else _sync_handler_probe(probe)
        )
    return {"operation": "handler", "probes": outputs}


def _sync_handler_probe(probe: Fields) -> Json:
    with _probe_patches(probe):
        run = _prepare(probe, "sync")
        run.obj.success_handler(
            result=_result(probe.get("result")),
            start_time=_datetime(probe.get("start_time"), "start_time"),
            end_time=_datetime(probe.get("end_time"), "end_time"),
            cache_hit=_optional_bool(probe.get("cache_hit")),
            standard_logging_object=probe.get("payload"),
        )
        return _handler_output(run)


async def _async_handler_probe(probe: Fields) -> Json:
    with _probe_patches(probe):
        run = _prepare(probe, "async")
        await run.obj.async_success_handler(
            result=_result(probe.get("result")),
            start_time=_datetime(probe.get("start_time"), "start_time"),
            end_time=_datetime(probe.get("end_time"), "end_time"),
            cache_hit=_optional_bool(probe.get("cache_hit")),
            standard_logging_object=probe.get("payload"),
        )
        return _handler_output(run)


def _handler_output(run: _Run) -> Json:
    return {
        "events": _events(run.records),
        "counters": cast("Json", [counter.recorded for counter in run.counters]),
        "context": {"trace_id": trace_id_var.get(), "session_id": session_id_var.get()},
        "details": _details(run.obj),
    }


def _sync_callbacks(probes: list[Fields]) -> Json:
    outputs: list[Json] = []
    for probe in probes:
        probe_with_params = cast("Fields", {**probe, "litellm_params": {"acompletion": True}})
        with _probe_patches(probe_with_params):
            run = _prepare(probe_with_params, "sync")
            run.obj.dynamic_success_callbacks = cast(  # pyright: ignore[reportAttributeAccessIssue]
                "list[str | Callable[..., object] | CustomLogger]",
                _strings(probe.get("dynamic_success"), "dynamic_success"),
            )
            known = _strings(probe.get("known"), "known")
            with patch.object(
                litellm,
                "_known_custom_logger_compatible_callbacks",
                known,
            ):
                should_run = run.obj._should_run_sync_callbacks_for_async_calls()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                ran = _run_sync_callbacks(run)
            outputs.append(
                {
                    "should_run": cast("Json", should_run),
                    "ran": ran,
                    "events": _events(run.records),
                }
            )
    return {"operation": "sync_callbacks", "probes": outputs}


def _run_sync_callbacks(run: _Run) -> bool:
    flags = [False]
    original = Logging.success_handler

    def recording(
        self: Logging,
        result: object = None,
        start_time: dt.datetime | None = None,
        end_time: dt.datetime | None = None,
        cache_hit: bool | None = None,  # noqa: FBT001
    ) -> None:
        flags[0] = True
        original(
            self,
            result=result,
            start_time=start_time,
            end_time=end_time,
            cache_hit=cache_hit,
        )

    with (
        patch.object(litellm_logging, "executor", _InlineExecutor()),
        patch.object(Logging, "success_handler", recording),
    ):
        run.obj.handle_sync_success_callbacks_for_async_calls(  # pyright: ignore[reportUnknownMemberType]
            result=_result({"kind": "chat"}),
            start_time=dt.datetime.fromisoformat("2024-01-01T00:00:00+00:00"),
            end_time=dt.datetime.fromisoformat("2024-01-01T00:00:01+00:00"),
        )
    return flags[0]


_OPERATIONS: Final[dict[str, Operation]] = {
    "assembled": _assembled,
    "recognized": _recognized,
    "dispatch": _dispatch,
    "handler": _handler,
    "sync_callbacks": _sync_callbacks,
}


def run(input_value: dict[str, Json]) -> Json:
    operations = _objects(input_value.get("operations"), "operations")
    return {"operations": [_operation(operation) for operation in operations]}


def _operation(operation: Fields) -> Json:
    name = _text(operation.get("operation"), "operation")
    handler = _OPERATIONS.get(name)
    if handler is None:
        raise InputError(f"unknown logging_success operation {name}")
    return handler(_objects(operation.get("probes"), "probes"))
