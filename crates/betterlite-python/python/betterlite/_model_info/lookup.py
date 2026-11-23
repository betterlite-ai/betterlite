import json
from typing import Final, NamedTuple, Protocol, TypeAlias, TypedDict

from pydantic import JsonValue, TypeAdapter

from betterlite import _native
from betterlite._model_info.catalog import _CATALOG
from betterlite.exceptions import ModelNotMappedError

CACHE_MAXSIZE: Final = 64

ModelInfo: TypeAlias = dict[str, JsonValue]

_MODEL_INFO: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class _CacheInfo(NamedTuple):
    hits: int
    misses: int
    maxsize: int
    currsize: int


class _CacheAccess(Protocol):
    def cache_clear(self) -> None: ...

    def cache_info(self) -> _CacheInfo: ...


class _ModelInfoRequest(TypedDict):
    model: str
    custom_llm_provider: str | None
    api_base: str | None
    api_key: str | None


class _ProviderRequest(TypedDict):
    model: str
    custom_llm_provider: str | None


class _CachedModelInfo(_CacheAccess):
    def __init__(self) -> None:
        self._lookups = 0

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> ModelInfo:
        _CATALOG.sync()
        self._lookups += 1
        request: _ModelInfoRequest = {
            "model": model,
            "custom_llm_provider": custom_llm_provider,
            "api_base": api_base,
            "api_key": api_key,
        }
        try:
            text = _CATALOG.native.model_info(json.dumps(request))
        except _native.ModelLookupError as error:
            raise ModelNotMappedError(str(error)) from error
        return _MODEL_INFO.validate_json(text)

    def cache_clear(self) -> None:
        self._lookups = 0

    def cache_info(self) -> _CacheInfo:
        return _CacheInfo(
            hits=0,
            misses=self._lookups,
            maxsize=CACHE_MAXSIZE,
            currsize=min(self._lookups, CACHE_MAXSIZE),
        )


get_model_info: Final = _CachedModelInfo()


def get_max_tokens(model: str) -> int | None:
    _CATALOG.sync()
    try:
        found = _CATALOG.native.max_tokens(model)
    except _native.ModelLookupError as error:
        raise ModelNotMappedError(str(error)) from error
    return found


def get_provider_info(
    model: str, custom_llm_provider: str | None
) -> dict[str, JsonValue] | None:
    _CATALOG.sync()
    request: _ProviderRequest = {
        "model": model,
        "custom_llm_provider": custom_llm_provider,
    }
    text = _CATALOG.native.provider_info(json.dumps(request))
    if text is None:
        return None
    return _MODEL_INFO.validate_json(text)
