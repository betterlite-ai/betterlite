import json
from typing import Final, NamedTuple, Protocol, TypeAlias, TypedDict

from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from betterlite import _native
from betterlite._model_info.catalog import catalog_state
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
        catalog_state.sync()
        self._lookups += 1
        request: _ModelInfoRequest = {
            "model": model,
            "custom_llm_provider": custom_llm_provider,
            "api_base": api_base,
            "api_key": api_key,
        }
        try:
            text = catalog_state.native.model_info(json.dumps(request))
        except _native.ModelLookupError as error:
            raise ModelNotMappedError(str(error)) from error
        return _MODEL_INFO.validate_json(text)

    @override
    def cache_clear(self) -> None:
        self._lookups = 0

    @override
    def cache_info(self) -> _CacheInfo:
        return _CacheInfo(
            hits=0,
            misses=self._lookups,
            maxsize=CACHE_MAXSIZE,
            currsize=min(self._lookups, CACHE_MAXSIZE),
        )


get_model_info: Final = _CachedModelInfo()


def get_max_tokens(model: str) -> int | None:
    catalog_state.sync()
    try:
        found = catalog_state.native.max_tokens(model)
    except _native.ModelLookupError as error:
        raise ModelNotMappedError(str(error)) from error
    return found


def get_provider_info(
    model: str, custom_llm_provider: str | None
) -> dict[str, JsonValue] | None:
    catalog_state.sync()
    request: _ProviderRequest = {
        "model": model,
        "custom_llm_provider": custom_llm_provider,
    }
    text = catalog_state.native.provider_info(json.dumps(request))
    if text is None:
        return None
    return _MODEL_INFO.validate_json(text)
