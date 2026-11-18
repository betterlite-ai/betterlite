import asyncio
import datetime as dt
from typing import Final, Literal, TypedDict

import litellm
from litellm.caching.caching import Cache
from litellm.caching.caching_handler import CachingHandlerResponse, LLMCachingHandler
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.s3_cache import S3Cache
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.caching import CachingSupportedCallTypes, LiteLLMCacheType
from litellm.types.utils import CachingDetails, EmbeddingResponse, ModelResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

litellm.suppress_debug_info = True
FIXED_TIME: Final = dt.datetime(2024, 1, 1, tzinfo=dt.UTC)
SUPPORTED: Final = TypeAdapter(list[CachingSupportedCallTypes] | None)
JSON: Final = TypeAdapter(JsonValue)


class Seed(TypedDict, total=False):
    key: str
    input: JsonValue
    value: JsonValue


class Scenario(TypedDict):
    operation: Literal["async", "sync"]
    model: str
    call_type: str
    original_function: str
    supported_call_types: list[str] | None
    cache_configured: bool
    cache_type: Literal["local", "s3"]
    kwargs: dict[str, JsonValue]
    entries: list[Seed]


class Input(TypedDict):
    scenarios: list[Scenario]


class NamedFunction:
    __name__: str

    def __init__(self, name: str) -> None:
        self.__name__ = name

    def __call__(self, model: JsonValue, messages: JsonValue) -> None: ...


class Params(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    acompletion: bool | None = None
    custom_llm_provider: str | None = None
    preset_cache_key: str | None = None


class Logged(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    model: str
    caching_details: CachingDetails | None
    litellm_params: Params


class HiddenFields(TypedDict, total=False):
    cache_hit: bool
    cache_key: str | None


class Hidden(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    params: HiddenFields = Field(alias="_hidden_params")


class EmbeddingDatum(TypedDict, total=False):
    embedding: list[float] | str
    index: int
    object: str


class UsageReply(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class EmbeddingReply(TypedDict):
    model: str | None
    data: list[EmbeddingDatum | None]
    usage: UsageReply | None


EMBEDDING: Final = TypeAdapter(EmbeddingReply)


class Seeded(InMemoryCache):
    def __init__(self) -> None:
        super().__init__()
        self.entries: dict[str, JsonValue] = {}

    @override
    async def async_get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.entries.get(key)

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.entries.get(key)


class SeededS3(S3Cache):
    def __init__(self, recorder: Seeded) -> None:
        self.recorder = recorder

    @override
    async def async_get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.recorder.entries.get(key)


def seed_key(cache: Cache, scenario: Scenario, entry: Seed) -> str:
    key = entry.get("key")
    if key is not None:
        return key
    request = entry.get("input")
    if request is not None:
        return cache.get_cache_key(**{**scenario["kwargs"], "input": request})
    return cache.get_cache_key(**scenario["kwargs"])


def scenario_cache(scenario: Scenario) -> Cache:
    supported = SUPPORTED.validate_python(scenario["supported_call_types"])
    cache = Cache(type=LiteLLMCacheType.LOCAL, supported_call_types=supported)
    seeded = Seeded()
    for entry in scenario["entries"]:
        seeded.entries[seed_key(cache, scenario, entry)] = entry["value"]
    cache.cache = SeededS3(seeded) if scenario["cache_type"] == "s3" else seeded
    return cache


def scenario_handler(scenario: Scenario) -> LLMCachingHandler:
    function = NamedFunction(scenario["original_function"])
    return LLMCachingHandler(function, scenario["kwargs"], FIXED_TIME)


def scenario_logging(scenario: Scenario) -> Logging:
    return Logging(
        model=scenario["model"],
        messages=None,
        stream=None,
        call_type=scenario["call_type"],
        start_time=FIXED_TIME,
        litellm_call_id="call-1",
        function_id="function-1",
    )


def duration_state(details: CachingDetails | None) -> str:
    if details is None:
        return "absent"
    return "null" if details["cache_duration_ms"] is None else "number"


def logging_report(logging_obj: Logging) -> dict[str, JsonValue]:
    logged = Logged.model_validate(logging_obj)
    details = logged.caching_details
    return {
        "model": logged.model,
        "provider": logged.litellm_params.custom_llm_provider,
        "acompletion": logged.litellm_params.acompletion,
        "preset_cache_key": logged.litellm_params.preset_cache_key,
        "cache_hit": None if details is None else details["cache_hit"],
        "cache_duration": duration_state(details),
    }


def hidden_fields(result: object) -> HiddenFields | None:
    try:
        return Hidden.model_validate(result).params
    except ValidationError:
        return None


def reply_report(reply: EmbeddingResponse) -> dict[str, JsonValue]:
    hidden = hidden_fields(reply) or {}
    return {
        "reply": EMBEDDING.validate_json(reply.model_dump_json()),
        "cache_hit": hidden.get("cache_hit"),
        "cache_key": hidden.get("cache_key"),
    }


def cached_report(cached_result: object | None) -> tuple[str, JsonValue, HiddenFields]:
    hidden = None if cached_result is None else hidden_fields(cached_result)
    if hidden is not None:
        return "converted", None, hidden
    if cached_result is None:
        return "none", None, {}
    return "raw", JSON.validate_python(cached_result), {}


def response_report(
    response: CachingHandlerResponse | None, logging_obj: Logging
) -> dict[str, JsonValue]:
    if response is None:
        return {
            "returned": False,
            "cached": "none",
            "raw": None,
            "cache_hit": None,
            "cache_key": None,
            "embedding": None,
            "all_cached": False,
            "uncached_input": None,
            "logging": logging_report(logging_obj),
        }
    kind, raw, hidden = cached_report(response.cached_result)
    reply = response.final_embedding_cached_response
    return {
        "returned": True,
        "cached": kind,
        "raw": raw,
        "cache_hit": hidden.get("cache_hit"),
        "cache_key": hidden.get("cache_key"),
        "embedding": None if reply is None else reply_report(reply),
        "all_cached": response.embedding_all_elements_cache_hit,
        "uncached_input": response.embedding_uncached_input,
        "logging": logging_report(logging_obj),
    }


async def async_scenario(scenario: Scenario) -> dict[str, JsonValue]:
    litellm.cache = scenario_cache(scenario) if scenario["cache_configured"] else None
    handler = scenario_handler(scenario)
    logging_obj = scenario_logging(scenario)
    response = await handler._async_get_cache(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L228
        model=scenario["model"],
        original_function=NamedFunction(scenario["original_function"]),
        logging_obj=logging_obj,
        start_time=FIXED_TIME,
        call_type=scenario["call_type"],
        kwargs=scenario["kwargs"],
        args=(),
    )
    return response_report(response, logging_obj)


def sync_scenario(scenario: Scenario) -> dict[str, JsonValue]:
    litellm.cache = scenario_cache(scenario) if scenario["cache_configured"] else None
    handler = scenario_handler(scenario)
    logging_obj = scenario_logging(scenario)
    response = handler._sync_get_cache(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching_handler.py#L368
        model=scenario["model"],
        original_function=NamedFunction(scenario["original_function"]),
        logging_obj=logging_obj,
        start_time=FIXED_TIME,
        call_type=scenario["call_type"],
        kwargs=scenario["kwargs"],
        args=(),
    )
    return response_report(response, logging_obj)


def run(input_value: Input) -> JsonValue:
    return [
        asyncio.run(async_scenario(scenario))
        if scenario["operation"] == "async"
        else sync_scenario(scenario)
        for scenario in input_value["scenarios"]
    ]
