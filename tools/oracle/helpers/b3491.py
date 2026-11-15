import asyncio
import threading
from typing import Literal, NotRequired, TypedDict, TypeVar

import httpx
from litellm.interactions.streaming_iterator import (
    InteractionsAPIStreamingIterator,
    SyncInteractionsAPIStreamingIterator,
)
from litellm.interactions.utils import get_provider_interactions_api_config
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.interactions.transformation import BaseInteractionsAPIConfig
from litellm.types.interactions.generated import (
    InteractionsAPIStreamingResponse as StreamEvent,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override

T = TypeVar("T")
JSON: TypeAdapter[Json] = TypeAdapter(Json)
EVENT: TypeAdapter[StreamEvent | None] = TypeAdapter(StreamEvent | None)

DISPATCH_TIMEOUT_SECONDS = 10.0


class Input(TypedDict):
    mode: Literal["async", "sync"]
    lines: list[str]
    model: NotRequired[str]
    litellm_params: NotRequired[dict[str, Json]]


class Dispatch(TypedDict):
    handler: str
    result: Json
    cache_hit: Json
    prefer_async_handlers: Json


class Output(TypedDict):
    events: list[Json]
    finished: bool
    completed: Json
    dispatches: list[Dispatch]


def dump(response: StreamEvent | None) -> Json:
    if response is None:
        return None
    return JSON.validate_json(response.model_dump_json())


class _Logging(Logging):
    def __init__(self, litellm_params: dict[str, Json]) -> None:
        self.model_call_details = {"litellm_params": litellm_params}
        self.dispatches: list[Dispatch] = []
        self.handled = threading.Event()

    def _record(self, handler: str, result: T | None, flags: dict[str, T]) -> None:
        dispatch: Dispatch = {
            "handler": handler,
            "result": dump(EVENT.validate_python(result)),
            "cache_hit": JSON.validate_python(flags.get("cache_hit")),
            "prefer_async_handlers": JSON.validate_python(flags.get("prefer_async_handlers")),
        }
        self.dispatches.append(dispatch)

    @override
    async def dispatch_success_handlers(
        self, result: T | None = None, *args: T, **kwargs: T
    ) -> None:
        del args
        self._record("dispatch_success_handlers", result, kwargs)

    @override
    async def async_success_handler(
        self, result: T | None = None, *args: T, **kwargs: T
    ) -> None:
        del args
        self._record("async_success_handler", result, kwargs)

    @override
    def success_handler(self, result: T | None = None, *args: T, **kwargs: T) -> None:
        del args
        self._record("success_handler", result, kwargs)
        self.handled.set()


async def collect_async(
    response: httpx.Response,
    model: str | None,
    config: BaseInteractionsAPIConfig,
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
    config: BaseInteractionsAPIConfig,
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
    config = get_provider_interactions_api_config("gemini")
    if config is None:
        raise RuntimeError("LiteLLM has no gemini interactions config")
    params = input_value.get("litellm_params")
    logging_obj = _Logging({} if params is None else params)
    model = input_value.get("model")
    if input_value["mode"] == "async":
        output = asyncio.run(collect_async(response, model, config, logging_obj))
    else:
        output = collect_sync(response, model, config, logging_obj)
    return JSON.validate_python(output)
