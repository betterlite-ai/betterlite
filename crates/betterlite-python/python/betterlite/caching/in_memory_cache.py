import json
from collections.abc import Callable
from typing import Final

from pydantic import JsonValue
from typing_extensions import override

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._native import NativeMemory
from betterlite.caching._backend import _BackendCache, number_value

DEFAULT_MAX_SIZE_IN_MEMORY: Final = 200
_DEFAULT_TTL: Final = 600
_DEFAULT_MAX_SIZE_PER_ITEM: Final = 1024


class InMemoryCache(_BackendCache[NativeMemory]):
    def __init__(
        self,
        max_size_in_memory: int | None = DEFAULT_MAX_SIZE_IN_MEMORY,
        default_ttl: int | None = _DEFAULT_TTL,
        max_size_per_item: int | None = _DEFAULT_MAX_SIZE_PER_ITEM,
        clock: Callable[[], float] | None = None,
    ) -> None:
        if clock is not None:
            raise UnsupportedCallArgument("clock")
        resolved_ttl = _DEFAULT_TTL if default_ttl is None else default_ttl
        super().__init__(default_ttl=resolved_ttl)
        self.max_size_in_memory = (
            DEFAULT_MAX_SIZE_IN_MEMORY if max_size_in_memory is None else max_size_in_memory
        )
        self.max_size_per_item = (
            _DEFAULT_MAX_SIZE_PER_ITEM if max_size_per_item is None else max_size_per_item
        )
        settings: dict[str, JsonValue] = {
            "max_size_in_memory": self.max_size_in_memory,
            "max_size_per_item": self.max_size_per_item,
        }
        # A composed DualCache reads the tier's own default from the native handle.
        if resolved_ttl != _DEFAULT_TTL:
            settings["default_ttl"] = resolved_ttl
        self.native = NativeMemory(json.dumps(settings))

    # An unset ttl writes with the tier's default_ttl, as LiteLLM's InMemoryCache does.
    @override
    def _write_ttl(self, kwargs: dict[str, JsonValue]) -> float | None:
        ttl = super()._write_ttl(kwargs)
        return float(self.default_ttl) if ttl is None else ttl

    def increment_cache(self, key: str, value: float, **kwargs: JsonValue) -> float:
        counted = number_value(self.native.increment(key, value, self._write_ttl(kwargs)))
        return float(counted)

    async def async_increment(self, key: str, value: float, **kwargs: JsonValue) -> float:
        counted = number_value(await self.native.async_increment(key, value, self._write_ttl(kwargs)))
        return float(counted)

    # LiteLLM answers the absolute expiry it stores, not the seconds left.
    async def async_get_ttl(self, key: str) -> float | None:
        text = await self.native.async_ttl(key)
        return None if text is None else float(text)
