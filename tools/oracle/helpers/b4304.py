import asyncio
import datetime as dt
from collections.abc import Callable
from typing import Literal, Protocol, Self, TypeAlias, TypedDict

import litellm
from litellm.caching.base_cache import BaseCache
from litellm.caching.caching import Cache
from litellm.caching.caching_handler import LLMCachingHandler
from litellm.caching.s3_cache import S3Cache
from litellm.types.caching import CachingSupportedCallTypes, LiteLLMCacheType
from litellm.types.utils import EmbeddingResponse, ModelResponse, TextCompletionResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

FIXED_TIME = dt.datetime(2024, 1, 1, tzinfo=dt.UTC)
CHAT_BODY: dict[str, JsonValue] = {
    "id": "chatcmpl-1",
    "created": 0,
    "object": "chat.completion",
    "model": "gpt-4",
    "choices": [],
}
CHAT_RESPONSE: TypeAdapter[ModelResponse] = TypeAdapter(ModelResponse)
EMBEDDING_RESPONSE: TypeAdapter[EmbeddingResponse] = TypeAdapter(EmbeddingResponse)
TEXT_COMPLETION_RESPONSE: TypeAdapter[TextCompletionResponse] = TypeAdapter(TextCompletionResponse)
SUPPORTED: TypeAdapter[list[CachingSupportedCallTypes] | None] = TypeAdapter(
    list[CachingSupportedCallTypes] | None
)


class AsyncStream:
    def __aiter__(self) -> Self:
        return self

    async def __anext__(self) -> JsonValue:
        raise StopAsyncIteration


class SyncStream:
    def __iter__(self) -> Self:
        return self

    def __next__(self) -> JsonValue:
        raise StopIteration


class NamedFunction:
    __name__: str

    def __init__(self, name: str) -> None:
        self.__name__ = name

    def __call__(self, model: JsonValue, messages: JsonValue) -> None: ...


class DualCacheLike(Protocol):
    async def async_set_cache(
        self, key: str, value: dict[str, JsonValue], **_kwargs: JsonValue
    ) -> None: ...

    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, dict[str, JsonValue]]], **_kwargs: JsonValue
    ) -> None: ...


CacheResult: TypeAlias = JsonValue | ModelResponse | EmbeddingResponse | TextCompletionResponse
StreamResult: TypeAlias = JsonValue | ModelResponse | AsyncStream | SyncStream


class Handler(Protocol):
    dual_cache: DualCacheLike | None

    async def async_set_cache(
        self,
        result: CacheResult,
        original_function: NamedFunction,
        kwargs: dict[str, JsonValue],
        args: tuple[JsonValue, ...] | None = None,
    ) -> None: ...

    def wrap_streaming_result_for_cache(  # ast-grep-ignore: no-long-names
        self, result: StreamResult, call_type: str
    ) -> StreamResult: ...


def build_handler(
    original_function: NamedFunction,
    request_kwargs: dict[str, JsonValue],
    start_time: dt.datetime,
) -> LLMCachingHandler:
    handler = LLMCachingHandler(original_function, {}, start_time)
    handler.request_kwargs.update(request_kwargs)
    return handler


HANDLER_FACTORY: Callable[
    [NamedFunction, dict[str, JsonValue], dt.datetime], Handler
] = TypeAdapter(
    Callable[[NamedFunction, dict[str, JsonValue], dt.datetime], Handler]
).validate_python(build_handler)


class StoreResult(TypedDict):
    kind: Literal["chat", "embedding", "raw", "text_completion"]
    body: dict[str, JsonValue]


class StoreScenario(TypedDict):
    function_name: str
    supported_call_types: list[CachingSupportedCallTypes] | None
    cache_configured: bool
    cache_type: Literal["local", "s3"]
    dual: bool
    kwargs: dict[str, JsonValue]
    args: list[JsonValue]
    result: StoreResult


class StoreInput(TypedDict):
    function: Literal["store"]
    scenarios: list[StoreScenario]


WrapResultKind: TypeAlias = Literal["async_stream", "sync_stream", "response"]


class WrapScenario(TypedDict):
    function_name: str
    call_type: str
    supported_call_types: list[CachingSupportedCallTypes] | None
    cache_configured: bool
    result_kind: WrapResultKind
    kwargs: dict[str, JsonValue]


class WrapInput(TypedDict):
    function: Literal["wrap"]
    scenarios: list[WrapScenario]


Input: TypeAlias = StoreInput | WrapInput


def write_record(sink: str, key: str, value: dict[str, JsonValue]) -> dict[str, JsonValue]:
    match value["response"]:
        case str():
            return {"sink": sink, "kind": "text", "key": key}
        case bool() | int() | float() | list() | dict() | None:
            return {"sink": sink, "kind": "value", "key": key}


class Recorder(BaseCache):
    def __init__(self, sink: str) -> None:
        super().__init__()
        self.sink = sink
        self.writes: list[JsonValue] = []

    @override
    async def async_set_cache(
        self, key: str, value: dict[str, JsonValue], **_kwargs: JsonValue
    ) -> None:
        self.writes.append(write_record(self.sink, key, value))

    @override
    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, dict[str, JsonValue]]], **_kwargs: JsonValue
    ) -> None:
        self.writes.extend(write_record(self.sink, key, value) for key, value in cache_list)


class RecordingS3(S3Cache):
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    @override
    async def async_set_cache(
        self, key: str, value: dict[str, JsonValue], **_kwargs: JsonValue
    ) -> None:
        await self.recorder.async_set_cache(key, value)

    @override
    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, dict[str, JsonValue]]], **_kwargs: JsonValue
    ) -> None:
        await self.recorder.async_set_cache_pipeline(cache_list)


def scenario_cache(scenario: StoreScenario, backend: Recorder) -> Cache:
    supported = SUPPORTED.validate_python(scenario["supported_call_types"])
    cache = Cache(type=LiteLLMCacheType.LOCAL, supported_call_types=supported)
    cache.cache = RecordingS3(backend) if scenario["cache_type"] == "s3" else backend
    return cache


def store_result(result: StoreResult) -> CacheResult:
    if result["kind"] == "chat":
        return CHAT_RESPONSE.validate_python(result["body"])
    if result["kind"] == "embedding":
        return EMBEDDING_RESPONSE.validate_python(result["body"])
    if result["kind"] == "text_completion":
        return TEXT_COMPLETION_RESPONSE.validate_python(result["body"])
    return result["body"]


def wrap_result(kind: WrapResultKind) -> StreamResult:
    if kind == "async_stream":
        return AsyncStream()
    if kind == "sync_stream":
        return SyncStream()
    return CHAT_RESPONSE.validate_python(CHAT_BODY)


async def store_scenario(scenario: StoreScenario) -> dict[str, JsonValue]:
    function = NamedFunction(scenario["function_name"])
    backend = Recorder("backend")
    dual = Recorder("dual")
    cache = scenario_cache(scenario, backend) if scenario["cache_configured"] else None
    litellm.cache = cache
    handler = HANDLER_FACTORY(function, scenario["kwargs"], FIXED_TIME)
    handler.dual_cache = dual if scenario["dual"] else None
    result = store_result(scenario["result"])
    await handler.async_set_cache(result, function, scenario["kwargs"], tuple(scenario["args"]))
    # create_cache_write_task runs the write on a later loop turn
    await asyncio.sleep(0)
    await asyncio.sleep(0)
    writes: list[JsonValue] = [*dual.writes, *backend.writes]
    return {"writes": writes}


def wrap_scenario(scenario: WrapScenario) -> JsonValue:
    function = NamedFunction(scenario["function_name"])
    supported = SUPPORTED.validate_python(scenario["supported_call_types"])
    litellm.cache = (
        Cache(type=LiteLLMCacheType.LOCAL, supported_call_types=supported)
        if scenario["cache_configured"]
        else None
    )
    handler = HANDLER_FACTORY(function, scenario["kwargs"], FIXED_TIME)
    result = wrap_result(scenario["result_kind"])
    wrapped = handler.wrap_streaming_result_for_cache(result, scenario["call_type"])
    return {"wrapped": wrapped is not result}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "store":
        stored: list[JsonValue] = [
            asyncio.run(store_scenario(scenario)) for scenario in input_value["scenarios"]
        ]
        return stored
    wrapped: list[JsonValue] = [
        wrap_scenario(scenario) for scenario in input_value["scenarios"]
    ]
    return wrapped
