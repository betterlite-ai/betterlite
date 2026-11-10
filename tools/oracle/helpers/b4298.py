import asyncio
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from types import SimpleNamespace
from typing import TypeAlias, cast
from unittest import mock

from litellm._logging import verbose_logger
from litellm.caching.base_cache import BaseCache
from litellm.caching.caching import Cache, CacheMode
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.types.caching import LiteLLMCacheType
from litellm.types.utils import Embedding, EmbeddingResponse, PromptTokensDetailsWrapper, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

NOW = 1_700_000_000.0
SKIP_MARKER = "skipping embedding cache write"
FAILURE_MARKER = "exception in add_cache"


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def optional_object(value: Json, name: str) -> dict[str, Json] | None:
    if value is None:
        return None
    return object_value(value, name)


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def number_value(value: Json, name: str) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise ValueError(f"{name} must be a number")
    return float(value)


def integer_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def flag_value(value: Json) -> bool:
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError("a flag must be a boolean")
    return value


def _request_kwargs(request: dict[str, Json]) -> dict[str, object]:
    kwargs: dict[str, object] = {}
    key = object_value(request.get("key"), "key")
    for entry in list_value(key.get("params", []), "params"):
        record = object_value(entry, "param")
        name = string_value(record.get("name"), "name")
        kwargs[name] = record.get("value")
    metadata = optional_object(key.get("metadata"), "metadata")
    if metadata is not None:
        kwargs["metadata"] = metadata
    litellm_metadata = optional_object(key.get("litellm_metadata"), "litellm_metadata")
    if litellm_metadata is not None:
        kwargs["litellm_metadata"] = litellm_metadata
    litellm_params = optional_object(key.get("litellm_params"), "litellm_params")
    if litellm_params is not None:
        kwargs["litellm_params"] = litellm_params
    control: dict[str, Json] = {}
    key_control = optional_object(key.get("cache_control"), "cache_control")
    if key_control is not None:
        control.update(key_control)
    envelope_control = optional_object(request.get("control"), "control")
    if envelope_control is not None:
        control.update(envelope_control)
    if control:
        kwargs["cache"] = control
    cache_key = optional_string(request.get("cache_key"), "cache_key")
    if cache_key is not None:
        kwargs["cache_key"] = cache_key
    return kwargs


def _cache(data: dict[str, Json], backend: BaseCache) -> Cache:
    cache_type = optional_string(data.get("cache_type"), "cache_type")
    cache_mode = optional_string(data.get("mode"), "mode")
    settings = optional_object(data.get("settings"), "settings")
    ttl = None if settings is None else settings.get("ttl")
    flush_size = None if settings is None else settings.get("redis_flush_size")
    return Cache(
        type=None if cache_type is None else LiteLLMCacheType(cache_type),
        mode=None if cache_mode is None else CacheMode(cache_mode),
        ttl=None if ttl is None else number_value(ttl, "ttl"),
        redis_flush_size=None
        if flush_size is None
        else integer_value(flush_size, "redis_flush_size"),
        _backend=backend,
    )


def _embedding_usage(value: dict[str, Json]) -> Usage:
    usage = Usage()
    tokens = value.get("prompt_tokens")
    setattr(  # noqa: B010 - the constructor turns None into 0
        usage,
        "prompt_tokens",
        None if tokens is None else integer_value(tokens, "prompt_tokens"),
    )
    details = optional_object(value.get("prompt_tokens_details"), "prompt_tokens_details")
    usage.prompt_tokens_details = (
        None if details is None else PromptTokensDetailsWrapper.model_validate(details)
    )
    return usage


def _embedding_response(value: dict[str, Json]) -> EmbeddingResponse:
    records = [
        Embedding.model_validate(object_value(entry, "record"))
        for entry in list_value(value.get("data", []), "data")
    ]
    response = EmbeddingResponse(model=optional_string(value.get("model"), "model"), data=records)
    usage = value.get("usage")
    response.usage = None if usage is None else _embedding_usage(object_value(usage, "usage"))
    return response


def _shaped_response(shape: str, item: dict[str, Json]) -> object:
    match shape:
        case "dict":
            return dict(item)
        case "pydantic":
            return Embedding.model_validate(item)
        case "object":
            return SimpleNamespace(**item)
        case _:
            raise ValueError(f"unknown response shape {shape}")


def _simple_cache() -> Cache:
    return Cache(type=LiteLLMCacheType.LOCAL, _backend=InMemoryCache())


def _convert(data: dict[str, Json]) -> Json:
    shape = string_value(data.get("shape"), "shape")
    response = _shaped_response(shape, object_value(data.get("response"), "response"))
    prompt_tokens = data.get("prompt_tokens")
    cached = _simple_cache()._convert_to_cached_embedding(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        response,
        optional_string(data.get("model"), "model"),
        prompt_tokens=None
        if prompt_tokens is None
        else integer_value(prompt_tokens, "prompt_tokens"),
        prompt_tokens_details=optional_object(
            data.get("prompt_tokens_details"), "prompt_tokens_details"
        ),
    )
    return cast(Json, {"cached": cached})


def _token_response(data: dict[str, Json]) -> EmbeddingResponse:
    usage_value = data.get("usage")
    response = EmbeddingResponse(
        model="text-embedding-3-small",
        data=[
            Embedding(embedding=[0.0], index=index, object="embedding")
            for index in range(integer_value(data.get("data_count"), "data_count"))
        ],
    )
    response.usage = (
        None if usage_value is None else _embedding_usage(object_value(usage_value, "usage"))
    )
    return response


def _prompt_tokens(data: dict[str, Json]) -> Json:
    response = _token_response(data)
    value = _simple_cache()._get_per_item_prompt_tokens(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        result=response,
        idx_in_result_data=integer_value(data.get("idx"), "idx"),
    )
    return cast(Json, {"prompt_tokens": value})


def _prompt_tokens_details(data: dict[str, Json]) -> Json:
    response = _token_response(data)
    value = _simple_cache()._get_per_item_prompt_tokens_details(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
        result=response,
        idx_in_result_data=integer_value(data.get("idx"), "idx"),
    )
    return cast(Json, {"details": value})


@dataclass
class PipelineCall:
    cache_list: list[tuple[str, Json]]
    ttl: Json


class RecordingBackend(BaseCache):
    def __init__(self, *, failure: bool) -> None:
        super().__init__()
        self.failure = failure
        self.pipeline: PipelineCall | None = None

    async def async_set_cache_pipeline(
        self, cache_list: Sequence[tuple[str, object]], **kwargs: object
    ) -> None:
        if self.failure:
            raise RuntimeError("cache pipeline failed")
        ttl: Json = cast(Json, kwargs.get("ttl"))
        self.pipeline = PipelineCall(
            cache_list=[(key, cast(Json, value)) for (key, value) in cache_list], ttl=ttl
        )


class FakeNative:
    def __init__(self, *, fail: bool, skip_after_first: bool) -> None:
        self.fail = fail
        self.skip_after_first = skip_after_first
        self.called = False
        self.calls = 0
        self.requests: list[Json] = []
        self.responses: list[Json] = []

    def request(
        self,
        cache: object,  # noqa: ARG002 - the runtime hands its facade along
        kwargs: Mapping[str, object],
    ) -> dict[str, object] | None:
        self.calls += 1
        if self.skip_after_first and self.calls > 1:
            return None
        cache_key = kwargs.get("cache_key")
        if not isinstance(cache_key, str):
            return None
        return {"cache_key": cache_key}

    async def async_store_batch(
        self, requests: Sequence[Json], responses: Sequence[Json]
    ) -> None:
        self.called = True
        if self.fail:
            raise RuntimeError("native batch storage failed")
        self.requests = list(requests)
        self.responses = list(responses)


def _native(value: dict[str, Json]) -> FakeNative:
    return FakeNative(
        fail=flag_value(value.get("fail")),
        skip_after_first=flag_value(value.get("skip_after_first")),
    )


def _add_entry(data: dict[str, Json]) -> Json:
    request = object_value(data.get("request"), "request")
    result = _embedding_response(object_value(data.get("result"), "result"))
    input_value = string_value(data.get("input"), "input")
    idx = integer_value(data.get("idx", 0), "idx")
    cache = _cache(data, RecordingBackend(failure=False))
    kwargs = _request_kwargs(request)
    with mock.patch("time.time", return_value=NOW):
        (cache_key, cached_data, updated) = cache.add_embedding_response_to_cache(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType, reportUnknownVariableType]
            result, input_value, kwargs, idx
        )
    return cast(
        Json,
        {
            "cache_key": cache_key,
            "cached": cast(Json, cached_data),
            "ttl": cast(dict[str, object], updated).get("ttl"),
        },
    )


def _run_pipeline(
    cache: Cache,
    result: EmbeddingResponse,
    dynamic: RecordingBackend | None,
    kwargs: dict[str, object],
) -> tuple[int, int]:
    errors: list[str] = []
    debugs: list[str] = []

    def capture_error(message: str, *args: object, **_kwargs: object) -> None:
        rendered = message % args if args else message
        if FAILURE_MARKER in rendered:
            errors.append(rendered)

    def capture_debug(message: str, *args: object, **_kwargs: object) -> None:
        rendered = message % args if args else message
        if SKIP_MARKER in rendered:
            debugs.append(rendered)

    with (
        mock.patch.object(verbose_logger, "debug", capture_debug),
        mock.patch.object(verbose_logger, "error", capture_error),
        mock.patch("time.time", return_value=NOW),
    ):
        asyncio.run(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
            cache.async_add_cache_pipeline(result, dynamic_cache_object=dynamic, **kwargs)  # pyright: ignore[reportUnknownMemberType]
        )
    return (len(errors), len(debugs))


def _pipeline(
    data: dict[str, Json],
) -> Json:
    request = object_value(data.get("request"), "request")
    result = _embedding_response(object_value(data.get("result"), "result"))
    backend = RecordingBackend(failure=flag_value(data.get("backend_failure")))
    cache = _cache(data, backend)
    dynamic = RecordingBackend(failure=False) if flag_value(data.get("dynamic")) else None
    native_value = optional_object(data.get("native"), "native")
    native = None if native_value is None else _native(native_value)
    if native is not None:
        setattr(cache, "_native_cache", native)  # noqa: B010 - the facade reads the runtime from this attribute
    (errors, debugs) = _run_pipeline(cache, result, dynamic, _request_kwargs(request))
    return _pipeline_output(backend, dynamic, native, errors, debugs)


def _pipeline_output(
    backend: RecordingBackend,
    dynamic: RecordingBackend | None,
    native: FakeNative | None,
    errors: int,
    debugs: int,
) -> Json:
    if native is not None:
        method: Json = "native" if native.called else None
    elif dynamic is not None and dynamic.pipeline is not None:
        method = "dynamic"
    elif backend.pipeline is not None:
        method = "default"
    else:
        method = None
    entries: list[Json] = []
    if native is None:
        source = dynamic if dynamic is not None else backend
        if source.pipeline is not None:
            entries = [
                {"key": key, "cached": cached, "ttl": source.pipeline.ttl}
                for (key, cached) in source.pipeline.cache_list
            ]
    native_output: Json = (
        None if native is None else {"requests": native.requests, "responses": native.responses}
    )
    return cast(
        Json,
        {
            "method": method,
            "entries": entries,
            "native": native_output,
            "errors": errors,
            "debugs": debugs,
        },
    )


def run(input_value: dict[str, Json]) -> Json:
    (function, payload) = next(iter(input_value.items()))
    if len(input_value) != 1:
        raise ValueError("a b4298 call names exactly one operation")
    data = {} if payload is None else object_value(payload, function)
    match function:
        case "convert_to_cached_embedding":
            return _convert(data)
        case "get_per_item_prompt_tokens":
            return _prompt_tokens(data)
        case "get_per_item_prompt_tokens_details":
            return _prompt_tokens_details(data)
        case "add_embedding_response_to_cache":
            return _add_entry(data)
        case "async_add_cache_pipeline":
            return _pipeline(data)
        case _:
            raise ValueError(f"unknown function {function}")
