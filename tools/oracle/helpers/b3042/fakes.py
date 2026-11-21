from datetime import UTC, datetime

from litellm.types.caching import RedisPipelineIncrementOperation
from pydantic import JsonValue
from typing_extensions import override

from .types import IncrementPipeline, UsageCache


def read_json(key: str, *, local_only: bool) -> JsonValue:
    record: dict[str, JsonValue] = {"key": key, "local_only": local_only}
    return record


def write_json(key: str, value: int | float | str, ttl: int, *, local_only: bool) -> JsonValue:
    record: dict[str, JsonValue] = {
        "key": key,
        "value": value,
        "ttl": ttl,
        "local_only": local_only,
    }
    return record


def increment_json(operation: RedisPipelineIncrementOperation) -> JsonValue:
    record: dict[str, JsonValue] = {
        "key": operation["key"],
        "increment_value": float(operation["increment_value"]),
        "ttl": operation["ttl"],
    }
    return record


class RecordingCache(UsageCache):
    def __init__(self, values: dict[str, int | float | str]) -> None:
        self.values = values
        self.reads: list[JsonValue] = []
        self.writes: list[JsonValue] = []
        self.increments: list[JsonValue] = []
        self.pipelines: list[JsonValue] = []
        self.dual_cache = RecordingDualCache(self)

    def record_increment(self, key: str, value: float, ttl: int | None) -> None:
        record = RedisPipelineIncrementOperation(key=key, increment_value=value, ttl=ttl)
        self.increments.append(increment_json(record))

    @override
    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> int | float | str | None:
        self.reads.append(read_json(key, local_only=local_only))
        return self.values.get(key)

    @override
    async def async_set_cache(
        self,
        key: str,
        value: int | float | str,
        ttl: int,
        *,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None:
        self.writes.append(write_json(key, value, ttl, local_only=local_only))
        self.values[key] = value

    @override
    async def async_increment_cache(
        self, key: str, value: float, *, litellm_parent_otel_span: JsonValue, ttl: int
    ) -> None:
        self.record_increment(key, value, ttl)


class RecordingDualCache(IncrementPipeline):
    redis_cache: JsonValue = None

    def __init__(self, owner: RecordingCache) -> None:
        self.owner = owner

    @override
    async def async_increment_cache(
        self, key: str, value: float, *, litellm_parent_otel_span: JsonValue, ttl: int
    ) -> None:
        self.owner.record_increment(key, value, ttl)

    @override
    async def async_increment_cache_pipeline(
        self,
        increment_list: list[RedisPipelineIncrementOperation],
        *,
        litellm_parent_otel_span: JsonValue,
    ) -> None:
        self.owner.pipelines.append([increment_json(operation) for operation in increment_list])


class FixedTime:
    def __init__(self, epoch: int) -> None:
        self.epoch = epoch

    def __call__(self) -> datetime:
        return datetime.fromtimestamp(self.epoch, tz=UTC)


class FixedFailClosed:
    def __init__(self, *, enabled: bool) -> None:
        self.enabled = enabled

    def __call__(self) -> bool:
        return self.enabled


class GroupMap:
    def __init__(self, groups: dict[str, str]) -> None:
        self.groups = groups

    def __call__(self, model: str) -> str | None:
        return self.groups.get(model)
