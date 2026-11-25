from abc import ABC, abstractmethod

from pydantic import JsonValue


class BaseCache(ABC):
    def __init__(self, default_ttl: int = 60) -> None:
        self.default_ttl = default_ttl

    def get_ttl(self, **kwargs: JsonValue) -> int | None:
        raw = kwargs.get("ttl")
        if raw is None:
            return self.default_ttl
        match raw:
            case bool() | int() | float() | str():
                try:
                    return int(raw)
                except ValueError:
                    return self.default_ttl
            case list() | dict():
                raise ValueError(f"ttl must be a number, not {type(raw).__name__}")

    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        raise NotImplementedError

    async def async_set_cache(
        self, key: str, value: JsonValue, **kwargs: JsonValue
    ) -> bool | None:
        raise NotImplementedError

    @abstractmethod
    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, JsonValue]], **kwargs: JsonValue
    ) -> None: ...

    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        raise NotImplementedError

    async def async_get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        raise NotImplementedError

    async def batch_cache_write(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        raise NotImplementedError

    async def disconnect(self) -> None:
        raise NotImplementedError

    async def test_connection(self) -> dict[str, JsonValue]:
        raise NotImplementedError
