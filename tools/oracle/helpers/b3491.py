import asyncio
import threading
from collections.abc import Callable, Coroutine
from datetime import datetime
from typing import Literal, NotRequired, TypedDict

import httpx
from litellm.interactions.streaming_iterator import (
    InteractionsAPIStreamingIterator,
    SyncInteractionsAPIStreamingIterator,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.interactions.transformation import GoogleAIStudioInteractionsConfig
from litellm.types.interactions.generated import InteractionsAPIStreamingResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

JSON: TypeAdapter[Json] = TypeAdapter(Json)

DISPATCH_TIMEOUT_SECONDS = 10.0


class Input(TypedDict):
    mode: Literal["async", "sync"]
    lines: list[str]
    model: NotRequired[str]
    litellm_params: NotRequired[dict[str, Json]]


class Dispatch(TypedDict):
    handler: str
    result: Json
    cache_hit: bool | None
    prefer_async_handlers: bool | None


class Output(TypedDict):
    events: list[Json]
    finished: bool
    completed: Json
    dispatches: list[Dispatch]


def dump(response: InteractionsAPIStreamingResponse | None) -> Json:
    if response is None:
        return None
    return JSON.validate_json(response.model_dump_json())


class _Logging(Logging):
    dispatch_success_handlers: Callable[..., Coroutine[None, None, None]]
    async_success_handler: Callable[..., Coroutine[None, None, None]]
    success_handler: Callable[..., None]

    def __init__(self, litellm_params: dict[str, Json], handled: threading.Event) -> None:
        self.model_call_details = {"litellm_params": litellm_params}
        self.dispatches: list[Dispatch] = []
        self.handled = handled
        self.dispatch_success_handlers = self._record_dispatch
        self.async_success_handler = self._record_async
        self.success_handler = self._record_sync

    async def _record_dispatch(
        self,
        result: InteractionsAPIStreamingResponse | None = None,
        start_time: datetime | None = None,
        end_time: datetime | None = None,
        cache_hit: bool | None = None,
        prefer_async_handlers: bool = False,
        **kwargs: Json,
    ) -> None:
        del start_time, end_time, kwargs
        dispatch: Dispatch = {
            "handler": "dispatch_success_handlers",
            "result": dump(result),
            "cache_hit": cache_hit,
            "prefer_async_handlers": prefer_async_handlers,
        }
        self.dispatches.append(dispatch)

    async def _record_async(
        self,
        result: InteractionsAPIStreamingResponse | None = None,
        start_time: datetime | None = None,
        end_time: datetime | None = None,
        cache_hit: bool | None = None,
        **kwargs: Json,
    ) -> None:
        del start_time, end_time, kwargs
        dispatch: Dispatch = {
            "handler": "async_success_handler",
            "result": dump(result),
            "cache_hit": cache_hit,
            "prefer_async_handlers": None,
        }
        self.dispatches.append(dispatch)

    def _record_sync(
        self,
        result: InteractionsAPIStreamingResponse | None = None,
        start_time: datetime | None = None,
        end_time: datetime | None = None,
        cache_hit: bool | None = None,
        **kwargs: Json,
    ) -> None:
        del start_time, end_time, kwargs
        dispatch: Dispatch = {
            "handler": "success_handler",
            "result": dump(result),
            "cache_hit": cache_hit,
            "prefer_async_handlers": None,
        }
        self.dispatches.append(dispatch)
        self.handled.set()


async def collect_async(
    response: httpx.Response,
    model: str | None,
    config: GoogleAIStudioInteractionsConfig,
    logging_obj: _Logging,
) -> Output:
    iterator = InteractionsAPIStreamingIterator(response, model, config, logging_obj)
    events: list[Json] = []
    while True:
        try:
            chunk = await iterator.__anext__()
        except StopAsyncIteration:
            break
        events.append(dump(chunk))
    await asyncio.sleep(0)
    await asyncio.sleep(0)
    return {
        "events": events,
        "finished": iterator.finished,
        "completed": dump(iterator.completed_response),
        "dispatches": logging_obj.dispatches,
    }


def collect_sync(
    response: httpx.Response,
    model: str | None,
    config: GoogleAIStudioInteractionsConfig,
    logging_obj: _Logging,
) -> Output:
    iterator = SyncInteractionsAPIStreamingIterator(response, model, config, logging_obj)
    events: list[Json] = []
    while True:
        try:
            chunk = iterator.__next__()
        except StopIteration:
            break
        events.append(dump(chunk))
    if iterator.completed_response is not None and not logging_obj.handled.wait(
        DISPATCH_TIMEOUT_SECONDS
    ):
        raise RuntimeError("the sync success handler did not run")
    return {
        "events": events,
        "finished": iterator.finished,
        "completed": dump(iterator.completed_response),
        "dispatches": logging_obj.dispatches,
    }


def run(input_value: Input) -> Json:
    lines = input_value["lines"]
    content = "" if not lines else "\n".join(lines) + "\n"
    response = httpx.Response(200, content=content.encode("utf-8"))
    config = GoogleAIStudioInteractionsConfig()
    params = input_value.get("litellm_params")
    handled = threading.Event()
    logging_obj = _Logging({} if params is None else params, handled)
    model = input_value.get("model")
    if input_value["mode"] == "async":
        output = asyncio.run(collect_async(response, model, config, logging_obj))
    else:
        output = collect_sync(response, model, config, logging_obj)
    return JSON.validate_python(output)
