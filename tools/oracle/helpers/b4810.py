import asyncio
import contextlib
from collections.abc import Awaitable, Callable, Coroutine, Mapping
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import TYPE_CHECKING, ClassVar, Protocol, TypeAlias, cast
from unittest.mock import patch

from a2a.compat.v0_3.types import SendStreamingMessageRequest, SendStreamingMessageResponse
from litellm.a2a_protocol import streaming_iterator as streaming_iterator_module
from litellm.a2a_protocol.main import (
    _a2a_cost_params,
    _build_streaming_logging_obj,
    _get_a2a_model_info,
    _set_agent_id_on_logging_obj,
    _set_usage_on_logging_obj,
)
from litellm.a2a_protocol.streaming_iterator import A2AStreamingIterator
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import BaseModel

if TYPE_CHECKING:
    from a2a.client import Client as A2AClientType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

# pyright: reportPrivateUsage=false


class LoggingObject(Protocol):
    model: str
    custom_llm_provider: str | None
    call_type: str
    stream: bool
    litellm_call_id: str
    function_id: str
    messages: Json
    litellm_params: dict[str, Json]
    optional_params: dict[str, Json]
    model_call_details: dict[str, Json]
    dispatch_success_handlers: Callable[..., Awaitable[None]]


def _as_object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _object(input_value: Mapping[str, Json], name: str) -> dict[str, Json]:
    return _as_object(input_value.get(name), name)


def _object_parameter(input_value: Mapping[str, Json], name: str) -> dict[str, object] | None:
    return cast(dict[str, object] | None, _optional_object(input_value, name))


def _optional_object(input_value: Mapping[str, Json], name: str) -> dict[str, Json] | None:
    value = input_value.get(name)
    return None if value is None else _as_object(value, name)


def _as_string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _string(input_value: Mapping[str, Json], name: str) -> str:
    return _as_string(input_value.get(name), name)


def _optional_string(input_value: Mapping[str, Json], name: str) -> str | None:
    value = input_value.get(name)
    return None if value is None else _as_string(value, name)


def _integer(input_value: Mapping[str, Json], name: str) -> int:
    value = input_value.get(name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def _boolean(input_value: Mapping[str, Json], name: str, *, default: bool) -> bool:
    value = input_value.get(name)
    if value is None:
        return default
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _result_usage(result: Json) -> Json:
    if not isinstance(result, dict):
        return result
    usage = result.get("usage")
    if not isinstance(usage, dict):
        return result
    complete = cast(dict[str, Json], usage)
    normalized: dict[str, Json] = {
        str(key): item for key, item in complete.items() if item is not None
    }
    return {**result, "usage": normalized}


def _jsonable(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast(dict[str, Json], value.model_dump(exclude_none=True))
    return cast(Json, value)


def _details(logging_obj: LoggingObject) -> dict[str, Json]:
    return {
        key: _jsonable(value)
        for key, value in logging_obj.model_call_details.items()
        if key != "litellm_trace_id"
    }


def _build_logging(details: dict[str, Json] | None = None) -> LoggingObject:
    logging_obj = cast(
        LoggingObject,
        Logging(
            model="a2a_agent/agent",
            messages=[{"role": "user", "content": "streaming-request"}],
            stream=False,
            call_type="asend_message_streaming",
            start_time=datetime(2026, 1, 1, tzinfo=UTC),
            litellm_call_id="call-1",
            function_id="function-1",
        ),
    )
    if details is not None:
        logging_obj.model_call_details = dict(details)
    return logging_obj


def _logging_handle(input_value: Mapping[str, Json]) -> LoggingObject | None:
    if not _boolean(input_value, "logging", default=True):
        return None
    return _build_logging(_optional_object(input_value, "details") or {})


def _kwargs(logging_obj: LoggingObject | None) -> dict[str, object]:
    return {} if logging_obj is None else {"litellm_logging_obj": logging_obj}


def _usage(input_value: Mapping[str, Json]) -> dict[str, Json]:
    logging_obj = _logging_handle(input_value)
    _set_usage_on_logging_obj(
        kwargs=_kwargs(logging_obj),
        prompt_tokens=_integer(input_value, "prompt_tokens"),
        completion_tokens=_integer(input_value, "completion_tokens"),
    )
    return {"details": None if logging_obj is None else _details(logging_obj)}


def _agent_id(input_value: Mapping[str, Json]) -> dict[str, Json]:
    logging_obj = _logging_handle(input_value)
    _set_agent_id_on_logging_obj(
        kwargs=_kwargs(logging_obj),
        agent_id=_optional_string(input_value, "agent_id"),
    )
    return {"details": None if logging_obj is None else _details(logging_obj)}


def _cost_params(input_value: Mapping[str, Json]) -> dict[str, Json]:
    filtered = _a2a_cost_params(_optional_object(input_value, "litellm_params"))
    return {"cost_params": cast(Json, dict(filtered))}


def _client(card: Mapping[str, Json] | None) -> "A2AClientType":
    agent_card = None if card is None else SimpleNamespace(name=card.get("name"))
    return cast("A2AClientType", SimpleNamespace(agent_card=agent_card))


def _model_info(input_value: Mapping[str, Json]) -> dict[str, Json]:
    client = _client(_optional_object(input_value, "card"))
    logging_obj = _logging_handle(input_value)
    agent_name = _get_a2a_model_info(client, _kwargs(logging_obj))
    return {
        "agent_name": agent_name,
        "model": None if logging_obj is None else logging_obj.model,
        "custom_llm_provider": None if logging_obj is None else logging_obj.custom_llm_provider,
        "details": None if logging_obj is None else _details(logging_obj),
    }


def _request(input_value: Mapping[str, Json]) -> SendStreamingMessageRequest:
    return SendStreamingMessageRequest.model_validate(_object(input_value, "request"))


def _build_streaming(input_value: Mapping[str, Json]) -> dict[str, Json]:
    logging_obj = cast(
        LoggingObject,
        _build_streaming_logging_obj(
            request=_request(input_value),
            agent_name=_string(input_value, "agent_name"),
            agent_id=_optional_string(input_value, "agent_id"),
            litellm_params=_object_parameter(input_value, "litellm_params"),
            metadata=_object_parameter(input_value, "metadata"),
            proxy_server_request=_object_parameter(input_value, "proxy_server_request"),
        ),
    )
    trace = _jsonable(logging_obj.model_call_details.get("litellm_trace_id"))
    return {
        "model": logging_obj.model,
        "custom_llm_provider": logging_obj.custom_llm_provider,
        "call_type": logging_obj.call_type,
        "stream": logging_obj.stream,
        "litellm_call_id": logging_obj.litellm_call_id,
        "function_id": logging_obj.function_id,
        "messages": _jsonable(logging_obj.messages),
        "litellm_params": _jsonable(logging_obj.litellm_params),
        "optional_params": _jsonable(logging_obj.optional_params),
        "details": _details(logging_obj),
        "trace_generated": isinstance(trace, str) and bool(trace),
    }


class _FakeDatetime:
    values: ClassVar[list[datetime]] = [datetime(2026, 1, 1, tzinfo=UTC)]
    index: ClassVar[int] = 0

    @classmethod
    def reset(cls, start: datetime, end: datetime) -> None:
        cls.values = [start, end]
        cls.index = 0

    @classmethod
    def now(cls) -> datetime:
        value = cls.values[min(cls.index, len(cls.values) - 1)]
        cls.index += 1
        return value


class _StreamError(Exception):
    pass


class _ScriptedStream:
    def __init__(self, chunks: list[SendStreamingMessageResponse], *, failure: bool) -> None:
        self.chunks = chunks
        self.failure = failure
        self.index = 0

    def __aiter__(self) -> "_ScriptedStream":
        return self

    async def __anext__(self) -> SendStreamingMessageResponse:
        if self.index >= len(self.chunks):
            if self.failure:
                raise _StreamError("the agent stream failed")
            raise StopAsyncIteration
        chunk = self.chunks[self.index]
        self.index += 1
        return chunk


class _DispatchRecorder:
    def __init__(self) -> None:
        self.calls: list[dict[str, Json]] = []

    async def dispatch(
        self,
        result: Json = None,
        start_time: datetime | None = None,
        end_time: datetime | None = None,
        cache_hit: Json = None,
        *,
        prefer_async_handlers: bool = False,
    ) -> None:
        self.calls.append(
            {
                "result": _result_usage(_jsonable(result)),
                "start_time": None if start_time is None else start_time.isoformat(),
                "end_time": None if end_time is None else end_time.isoformat(),
                "cache_hit": cache_hit,
                "prefer_async_handlers": prefer_async_handlers,
            }
        )


class _CreateTask:
    def __init__(self) -> None:
        self.calls = 0

    def __call__(self, coro: Coroutine[None, None, None]) -> None:
        self.calls += 1
        with contextlib.suppress(StopIteration):
            coro.send(None)


async def _iterate_scenario(scenario: Mapping[str, Json]) -> dict[str, Json]:
    request = SendStreamingMessageRequest.model_validate(_object(scenario, "request"))
    logging_obj = _build_logging(_object(scenario, "logging"))
    chunks = [
        SendStreamingMessageResponse.model_validate(item)
        for item in cast(list[Json], scenario.get("chunks", []))
    ]
    recorder = _DispatchRecorder()
    logging_obj.dispatch_success_handlers = recorder.dispatch
    task = _CreateTask()
    yielded = 0
    error: str | None = None
    with (
        patch.object(streaming_iterator_module, "datetime", _FakeDatetime),
        patch.object(asyncio, "create_task", task),
    ):
        iterator = A2AStreamingIterator(
            stream=_ScriptedStream(
                chunks, failure=_boolean(scenario, "failure", default=False)
            ),
            request=request,
            logging_obj=cast(Logging, logging_obj),
            agent_name=_string(scenario, "agent_name"),
        )
        while True:
            try:
                await iterator.__anext__()
            except StopAsyncIteration:
                break
            except _StreamError:
                error = "stream"
                break
            yielded += 1
    dumped = [
        cast(Json, chunk.model_dump(mode="json", exclude_none=True)) for chunk in iterator.chunks
    ]
    final = (
        None
        if iterator.final_chunk is None
        else iterator.final_chunk.model_dump(mode="json", exclude_none=True)
    )
    return {
        "yielded": yielded,
        "chunks": dumped,
        "texts": cast(Json, list(iterator.collected_text_parts)),
        "final_chunk": cast(Json, final),
        "details": _details(logging_obj),
        "dispatch": cast(Json, recorder.calls[0]) if recorder.calls else None,
        "error": error,
    }


async def _iterate(input_value: Mapping[str, Json]) -> Json:
    scenarios: list[Json] = []
    for scenario in _instances(input_value):
        start = datetime.fromisoformat(_string(scenario, "start_time"))
        end = datetime.fromisoformat(_string(scenario, "end_time"))
        _FakeDatetime.reset(start, end)
        scenarios.append(cast(Json, await _iterate_scenario(scenario)))
    return {"results": scenarios}


def _instances(input_value: Mapping[str, Json]) -> list[Mapping[str, Json]]:
    instances = cast(list[Json], input_value.get("instances", []))
    checked: list[Mapping[str, Json]] = []
    for instance in instances:
        if not isinstance(instance, dict):
            raise ValueError("each instance must be an object")
        checked.append(instance)
    return checked


def _sequence(
    input_value: Mapping[str, Json],
    handler: Callable[[Mapping[str, Json]], dict[str, Json]],
) -> dict[str, Json]:
    results: list[Json] = [cast(Json, handler(instance)) for instance in _instances(input_value)]
    return {"results": results}


def run(input_value: dict[str, Json]) -> Json:
    function = _string(input_value, "function")
    if function == "iterate":
        return asyncio.run(_iterate(input_value))
    handlers: dict[str, Callable[[Mapping[str, Json]], dict[str, Json]]] = {
        "set_usage": _usage,
        "set_agent_id": _agent_id,
        "cost_params": _cost_params,
        "model_info": _model_info,
        "build_streaming": _build_streaming,
    }
    handler = handlers.get(function)
    if handler is None:
        raise ValueError(f"unknown b4810 function: {function}")
    return _sequence(input_value, handler)
