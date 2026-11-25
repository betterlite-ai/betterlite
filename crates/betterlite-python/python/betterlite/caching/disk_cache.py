import json

from pydantic import JsonValue

from betterlite._native import NativeDisk
from betterlite.caching._backend import BackendCache, number_value, ttl_of


class DiskCache(BackendCache[NativeDisk]):
    def __init__(self, disk_cache_dir: str | None = None) -> None:
        super().__init__()
        self.disk_cache_dir = disk_cache_dir
        settings: dict[str, JsonValue] = {}
        if disk_cache_dir is not None:
            settings["disk_cache_dir"] = disk_cache_dir
        self.native = NativeDisk(json.dumps(settings))

    def increment_cache(self, key: str, value: int, **kwargs: JsonValue) -> int:
        return int(number_value(self.native.increment(key, value, ttl_of(kwargs))))

    async def async_increment(self, key: str, value: int, **kwargs: JsonValue) -> int:
        return int(number_value(await self.native.async_increment(key, value, ttl_of(kwargs))))
