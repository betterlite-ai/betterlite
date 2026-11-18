import asyncio
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.spend_counter_reseed import SpendCounterReseed
from litellm.proxy.spend_tracking.spend_counter_batch import (
    active_spend_counter_batch,
    spend_counter_batch_scope,
)
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

FLOAT: Final = TypeAdapter(float)
JSON: Final = TypeAdapter(JsonValue)

Operation: TypeAlias = Literal[
    "from_db",
    "end_user_from_db",
    "increment_in_memory",
    "coalesced",
    "window_from_table",
    "window_from_db",
    "window_from_spend_logs",
]


class CacheInput(TypedDict):
    redis: bool
    answers: list[JsonValue]
    set_nx: Literal["stored", "taken", "fail"]
    memory: float | None
    memory_set_fails: bool


class Input(TypedDict):
    op: Operation
    counter_key: str
    prisma: bool
    row_spend: float | None
    entity: str
    entity_id: str
    duration: str | None
    start: str
    expected_start: str
    window: tuple[str, float] | None
    logs: list[tuple[str, float]]
    fails: bool
    cache: CacheInput
    batch: JsonValue
    increment: float
    require_cache_warm: bool


class LogWhere(TypedDict):
    startTime: dict[str, datetime]
    api_key: NotRequired[str]
    team_id: NotRequired[str]


class Row:
    def __init__(self, spend: float | None, window_start: datetime | None = None) -> None:
        self.spend = spend
        self.window_start = window_start


def as_utc(moment: datetime) -> datetime:
    if moment.tzinfo is not None:
        return moment
    return moment.replace(tzinfo=UTC)


def scripted_amount(answer: JsonValue) -> float | None:
    if answer == "miss":
        return None
    if answer == "fail":
        raise ValueError("the scripted cache read failed")
    return FLOAT.validate_python(answer)


def required_duration(duration: str | None) -> str:
    if duration is None:
        raise ValueError("window_from_table needs a duration")
    return duration


class ScriptedTable:
    def __init__(
        self,
        row: Row | None,
        window: Row | None,
        logs: list[tuple[datetime, float]],
        *,
        fails: bool,
    ) -> None:
        self.row = row
        self.window = window
        self.logs = logs
        self.fails = fails

    async def find_unique(self, where: dict[str, JsonValue]) -> Row | None:
        if self.fails:
            raise ValueError("the scripted store read failed")
        if "entity_type_entity_id_window_duration" in where:
            return self.window
        return self.row

    async def group_by(
        self,
        by: list[str],
        where: LogWhere,
        **kwargs: JsonValue,
    ) -> list[dict[str, JsonValue]]:
        del kwargs
        if self.fails:
            raise ValueError("the scripted store read failed")
        field = by[0]
        entity_id = where["api_key"] if field == "api_key" else where["team_id"]
        window_start = where["startTime"]["gte"]
        matching = [spend for moment, spend in self.logs if as_utc(moment) >= as_utc(window_start)]
        if not matching:
            return []
        return [{field: entity_id, "_sum": {"spend": sum(matching)}}]


class FakeDb(PrismaWrapper):
    def __init__(self, table: ScriptedTable) -> None:
        self.table = table

    @override
    def __getattr__(self, name: str) -> ScriptedTable:
        del name
        return self.table


class FakeClient(PrismaClient):
    def __init__(self, table: ScriptedTable) -> None:
        self.db = FakeDb(table)


class ScriptedMemory(InMemoryCache):
    def __init__(self, initial: float | None, *, fails: bool) -> None:
        super().__init__()
        self.initial = initial
        self.fails = fails
        self.values: dict[str, float] = {}

    def current(self, key: str) -> float | None:
        if key in self.values:
            return self.values[key]
        return self.initial

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> float | None:
        del kwargs
        return self.current(key)

    @override
    def set_cache(self, key: str, value: float, **kwargs: JsonValue) -> None:
        del kwargs
        if self.fails:
            raise ValueError("the scripted memory write failed")
        self.values[key] = value


class ScriptedRedis(RedisCache):
    # RedisCache.__init__ opens a real client; the scripted methods below
    # answer every read and write
    def __init__(self, cache: CacheInput, batch: JsonValue) -> None:
        object.__init__(self)
        self.answers = list(cache["answers"])
        self.mode = cache["set_nx"]
        self.batch = batch
        self.sets: list[tuple[str, float]] = []

    @override
    async def async_get_cache(self, key: str, **kwargs: JsonValue) -> float | None:
        del key, kwargs
        if not self.answers:
            return None
        return scripted_amount(self.answers.pop(0))

    @override
    async def async_set_cache(self, key: str, value: float, **kwargs: JsonValue) -> bool:
        del kwargs
        if self.mode == "fail":
            raise ValueError("the scripted redis write failed")
        self.sets.append((key, value))
        return self.mode == "stored"

    @override
    async def async_batch_get_cache(
        self,
        key_list: list[str],
        **kwargs: JsonValue,
    ) -> dict[str, float | None]:
        del kwargs
        return dict.fromkeys(key_list, scripted_amount(self.batch))


def build_table(data: Input) -> ScriptedTable:
    window_input = data["window"]
    window: Row | None = None
    if window_input is not None:
        window = Row(float(window_input[1]), datetime.fromisoformat(window_input[0]))
    logs = [(datetime.fromisoformat(start), float(spend)) for start, spend in data["logs"]]
    spend = data["row_spend"]
    row = None if spend is None else Row(float(spend))
    return ScriptedTable(row, window, logs, fails=data["fails"])


async def apply(data: Input, cache: DualCache, client: FakeClient | None) -> float | None:
    value: float | None = None
    operation = data["op"]
    match operation:
        case "from_db":
            value = await SpendCounterReseed.from_db(client, data["counter_key"])
        case "end_user_from_db":
            value = await SpendCounterReseed.end_user_from_db(client, data["counter_key"])
        case "increment_in_memory":
            value = await SpendCounterReseed.increment_in_memory(
                cache, data["counter_key"], data["increment"]
            )
        case "coalesced":
            value = await SpendCounterReseed.coalesced(
                client, cache, data["counter_key"], data["require_cache_warm"]
            )
        case "window_from_table":
            value = await SpendCounterReseed.window_from_table(
                client,
                data["entity"],
                data["entity_id"],
                required_duration(data["duration"]),
                datetime.fromisoformat(data["expected_start"]),
            )
        case "window_from_db":
            value = await SpendCounterReseed.window_from_db(
                client,
                data["entity"],
                data["entity_id"],
                data["duration"],
                datetime.fromisoformat(data["expected_start"]),
            )
        case "window_from_spend_logs":
            value = await SpendCounterReseed.window_from_spend_logs(
                client,
                data["entity"],
                data["entity_id"],
                datetime.fromisoformat(data["start"]),
            )
    return value


async def scenario(data: Input) -> JsonValue:
    cache_input = data["cache"]
    memory = ScriptedMemory(cache_input["memory"], fails=cache_input["memory_set_fails"])
    redis_enabled = cache_input["redis"]
    redis = ScriptedRedis(cache_input, data["batch"])
    cache = DualCache(in_memory_cache=memory, redis_cache=redis if redis_enabled else None)
    table = build_table(data)
    client = FakeClient(table) if data["prisma"] else None
    value: float | None = None
    batch_after: float | None = None
    if data["batch"] is None:
        value = await apply(data, cache, client)
    else:
        batch_redis: Final = redis if redis_enabled else None
        with spend_counter_batch_scope(batch_redis, frozenset((data["counter_key"],))):
            value = await apply(data, cache, client)
            active = active_spend_counter_batch()
            if active is not None:
                state = await active.read(data["counter_key"])
                if state is not None:
                    batch_after = state[0]
    return JSON.validate_python(
        {
            "value": value,
            "memory_after": memory.current(data["counter_key"]),
            "batch_after": batch_after,
            "redis_sets": [JSON.validate_python([key, amount]) for key, amount in redis.sets],
        }
    )


def run(input_value: Input) -> JsonValue:
    return asyncio.run(scenario(input_value))
