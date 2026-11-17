import asyncio
import base64
import datetime as dt
import logging
from collections.abc import AsyncIterator, Mapping
from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.caching.base_cache import BaseCache
from litellm.caching.caching import (
    DEFAULT_CACHING_SUPPORTED_CALL_TYPES as SUPPORTED_CALL_TYPES,
)
from litellm.caching.caching import (
    Cache,
)
from litellm.caching.caching_handler import LLMCachingHandler
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.logging_worker import GLOBAL_LOGGING_WORKER
from litellm.llms.anthropic.pass_through.messages.response_cache import (
    AnthropicMessagesStreamCacheWriter,
    convert_cached_anthropic_messages_result,
)
from litellm.llms.anthropic.pass_through.messages.response_cache import (
    CachedAnthropicMessagesStreamIterator as CachedStreamIterator,
)
from litellm.proxy.anthropic_endpoints.streaming_model_restamp import (
    AnthropicStreamModelRestamper,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

LOGGER_NAME: Final = "LiteLLM"
REQUEST_MODEL: Final = "claude-3"
ORIGINAL_FUNCTION: Final = "aanthropic_messages"
FIXED_TIME: Final = dt.datetime(2026, 1, 1, tzinfo=dt.UTC)
SETTLE_TURNS: Final = 8

PROCESSED: TypeAdapter[bytes | str | dict[str, JsonValue]] = TypeAdapter(
    bytes | str | dict[str, JsonValue]
)


class EventChunk(TypedDict):
    kind: Literal["event"]
    value: dict[str, JsonValue]


class BytesChunk(TypedDict):
    kind: Literal["bytes"]
    value: str


class TextChunk(TypedDict):
    kind: Literal["text"]
    value: str


Chunk: TypeAlias = EventChunk | BytesChunk | TextChunk


class RestampInput(TypedDict):
    requested_model: str
    chunks: list[Chunk]


class CacheStreamInput(TypedDict):
    reads: list[str]
    preset_cache_key: str | None
    fail: str | None
    replay: bool


class Input(TypedDict):
    restamp: RestampInput | None
    cache_stream: CacheStreamInput | None


class Write(TypedDict):
    payload: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]


class CallDetails(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    cache_hit: bool | None = None


class LoggedCall(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: CallDetails


async def aanthropic_messages() -> None: ...


class WarningCollector(logging.Handler):
    def __init__(self, events: list[JsonValue]) -> None:
        super().__init__(level=logging.WARNING)
        self.events = events

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.events.append({"event": "warning", "level": record.levelname.lower()})


class RecordingCache(Cache):
    def __init__(self, fail: str | None) -> None:
        self.supported_call_types = list(SUPPORTED_CALL_TYPES)
        self.fail = fail
        self.writes: list[Write] = []

    @override
    async def async_add_cache(
        self,
        result: dict[str, JsonValue],
        dynamic_cache_object: BaseCache | None = None,
        **kwargs: JsonValue,
    ) -> None:
        self.writes.append({"payload": result, "kwargs": dict(kwargs)})
        if self.fail is not None:
            raise RuntimeError(self.fail)


def chunk_input(chunk: Chunk) -> bytes | str | dict[str, JsonValue]:
    match chunk["kind"]:
        case "event":
            return chunk["value"]
        case "bytes":
            return base64.b64decode(chunk["value"])
        case "text":
            return chunk["value"]


def chunk_view(chunk: bytes | str | dict[str, JsonValue]) -> dict[str, JsonValue]:
    match chunk:
        case bytes():
            return {"kind": "bytes", "value": base64.b64encode(chunk).decode("ascii")}
        case str():
            return {"kind": "text", "value": chunk}
        case dict():
            return {"kind": "event", "value": chunk}


def restamp_answer(input_value: RestampInput) -> JsonValue:
    restamper = AnthropicStreamModelRestamper(input_value["requested_model"])
    chunks: list[JsonValue] = [
        chunk_view(PROCESSED.validate_python(restamper.process(chunk_input(chunk))))
        for chunk in input_value["chunks"]
    ]
    return {"chunks": chunks, "flushed": chunk_view(restamper.flush())}


def make_logging() -> Logging:
    return Logging(
        model=REQUEST_MODEL,
        messages=None,
        stream=True,
        call_type=ORIGINAL_FUNCTION,
        start_time=FIXED_TIME,
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def cached_hit(logging_obj: Logging) -> bool:
    details = LoggedCall.model_validate(logging_obj).model_call_details
    return details.cache_hit is True


async def feed(reads: list[str]) -> AsyncIterator[bytes]:
    for read in reads:
        yield read.encode("utf-8")


async def drain_writer(
    input_value: CacheStreamInput, cache: RecordingCache
) -> list[JsonValue]:
    handler = LLMCachingHandler(aanthropic_messages, {}, FIXED_TIME)
    handler.request_kwargs.update({"model": REQUEST_MODEL, "stream": True})
    handler.preset_cache_key = input_value["preset_cache_key"]
    litellm.cache = cache
    writer = AnthropicMessagesStreamCacheWriter(
        stream=feed(input_value["reads"]), caching_handler=handler
    )
    yielded: list[JsonValue] = []
    async for chunk in writer:
        match chunk:
            case bytes():
                yielded.append(chunk.decode("utf-8"))
            case str():
                yielded.append(chunk)
    for _ in range(SETTLE_TURNS):
        if cache.writes:
            break
        await asyncio.sleep(0)
    return yielded


async def replay_written(
    input_value: CacheStreamInput, cache: RecordingCache
) -> tuple[list[JsonValue] | None, bool]:
    if not input_value["replay"] or not cache.writes:
        return (None, False)
    logging_obj = make_logging()
    converted = convert_cached_anthropic_messages_result(
        cache.writes[0]["payload"], logging_obj, {"model": REQUEST_MODEL}
    )
    match converted:
        case CachedStreamIterator():
            hit = cached_hit(logging_obj)
            replayed: list[JsonValue] = [chunk.decode("utf-8") async for chunk in converted]
            await GLOBAL_LOGGING_WORKER.stop()
            return (replayed, hit)
        case Mapping():
            return (None, False)


async def cache_answer(input_value: CacheStreamInput) -> JsonValue:
    cache = RecordingCache(input_value["fail"])
    events: list[JsonValue] = []
    collector = WarningCollector(events)
    logger = logging.getLogger(LOGGER_NAME)
    logger.addHandler(collector)
    try:
        yielded = await drain_writer(input_value, cache)
    finally:
        logger.removeHandler(collector)
    replayed, hit = await replay_written(input_value, cache)
    writes: list[JsonValue] = [
        {"payload": write["payload"], "kwargs": write["kwargs"]} for write in cache.writes
    ]
    return {
        "yielded": yielded,
        "writes": writes,
        "events": events,
        "replayed": replayed,
        "cache_hit": hit,
    }


def run(input_value: Input) -> JsonValue:
    restamp = input_value["restamp"]
    cache_stream = input_value["cache_stream"]
    return {
        "restamp": None if restamp is None else restamp_answer(restamp),
        "cache_stream": (
            None if cache_stream is None else asyncio.run(cache_answer(cache_stream))
        ),
    }
