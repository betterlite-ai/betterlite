import asyncio
import datetime as dt
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.caching.caching import Cache
from litellm.caching.caching_handler import (
    LLMCachingHandler,
    _current_format_embedding_entry,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L186
)
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.types.caching import CachingSupportedCallTypes, LiteLLMCacheType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
FIXED_TIME = dt.datetime(2024, 1, 1, tzinfo=dt.UTC)


class EligibilityInput(TypedDict):
    function: Literal["eligibility"]
    call_type: str
    supported_call_types: list[str] | None
    cache: Json


class SeedEntry(TypedDict):
    input: Json
    record: Json


class RetrieveInput(TypedDict):
    function: Literal["retrieve"]
    call_type: str
    kwargs: dict[str, Json]
    parameters: list[str]
    args: list[Json]
    entries: list[SeedEntry]


class EntryInput(TypedDict):
    function: Literal["current_format_embedding_entry"]
    entries: list[Json]


Input = EligibilityInput | RetrieveInput | EntryInput


class FixedClock:
    def __call__(self) -> float:
        return 0.0


class NamedFunction:
    __name__: str

    def __init__(self, name: str) -> None:
        self.__name__ = name

    def __call__(self, model: Json, messages: Json) -> None:
        ...


class Handler(LLMCachingHandler):
    def __init__(self, call_type: str, kwargs: dict[str, Json]) -> None:
        self.call_type = call_type
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L206
            original_function=NamedFunction(call_type),
            request_kwargs=cast(dict[str, object], kwargs),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L209
            start_time=FIXED_TIME,
        )

    def call_type_supported(self) -> bool:
        return self._is_call_type_supported_by_cache(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L1156
            original_function=NamedFunction(self.call_type),
        )

    def stores_result(self, kwargs: dict[str, Json]) -> bool:
        return self._should_store_result_in_cache(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L1125
            original_function=NamedFunction(self.call_type),
            kwargs=kwargs,
        )

    async def cached_result(
        self,
        call_type: str,
        kwargs: dict[str, Json],
        args: tuple[Json, ...],
    ) -> Json:
        typed_kwargs = cast(dict[str, object], kwargs)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L209
        cached = await self._retrieve_from_cache(
            call_type=call_type,
            kwargs=typed_kwargs,
            args=args,
        )
        return cast(Json, cached)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L757-L832


def local_cache(supported: list[CachingSupportedCallTypes] | None) -> Cache:
    cache = Cache(type=LiteLLMCacheType.LOCAL, supported_call_types=supported)
    cache.cache = InMemoryCache(clock=FixedClock())
    return cache


def eligibility(input_value: EligibilityInput) -> Json:
    supported = input_value["supported_call_types"]
    typed_supported: list[CachingSupportedCallTypes] | None
    if supported is None:
        typed_supported = None
    else:
        typed_supported = cast(list[CachingSupportedCallTypes], supported)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/caching.py#L26
    litellm.cache = local_cache(typed_supported)
    handler = Handler(input_value["call_type"], {"cache": input_value["cache"]})
    result: dict[str, Json] = {
        "supported": handler.call_type_supported(),
        "store": handler.stores_result({"cache": input_value["cache"]}),
    }
    return result


def format_entries(input_value: EntryInput) -> Json:
    records: list[Json] = [
        cast(Json, _current_format_embedding_entry(entry))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L186-L202
        for entry in input_value["entries"]
    ]
    return records


async def seed_entries(cache: Cache, kwargs: dict[str, Json], entries: list[SeedEntry]) -> None:
    for entry in entries:
        key_kwargs = dict(kwargs)
        key_kwargs["input"] = entry["input"]
        key = cache.get_cache_key(**key_kwargs)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L358
        value: Json = {"timestamp": 0.0, "response": entry["record"]}
        await cache.cache.async_set_cache(key, value)  # type: ignore[no-untyped-call, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/base_cache.py#L38


async def retrieve_async(input_value: RetrieveInput) -> Json:
    kwargs = dict(input_value["kwargs"])
    cache = local_cache(None)
    litellm.cache = cache
    await seed_entries(cache, kwargs, input_value["entries"])
    handler = Handler(input_value["call_type"], kwargs)
    cached = await handler.cached_result(
        input_value["call_type"],
        kwargs,
        tuple(input_value["args"]),
    )
    result: dict[str, Json] = {
        "cached": cached,
        "preset_cache_key": handler.preset_cache_key,
        "request_kwargs": cast(Json, handler.request_kwargs),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L216
    }
    return result


def run(input_value: Input) -> Json:
    match input_value["function"]:
        case "eligibility":
            return eligibility(input_value)
        case "current_format_embedding_entry":
            return format_entries(input_value)
        case "retrieve":
            return asyncio.run(retrieve_async(input_value))
        case _:  # pyright: ignore[reportUnnecessaryComparison]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L71
            raise ValueError("unknown caching handler function")
