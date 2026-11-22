import asyncio
from dataclasses import dataclass
from datetime import datetime
from typing import Literal, TypedDict

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.proxy import proxy_server
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.spend_tracking.spend_counter_batch import (
    active_spend_counter_batch,
    spend_counter_batch_scope,
)
from litellm.proxy.utils import PrismaClient
from opentelemetry.trace import Span
from pydantic import JsonValue
from typing_extensions import override


class RowsInput(TypedDict):
    primary: float | None
    end_user: float | None
    window: float | None
    fails: bool
    reset_marker: float | None


class Input(TypedDict):
    op: Literal["get_current_spend", "read_spend_counter_cache_value"]
    counter_key: str
    fallback_spend: float
    max_budget: float | None
    window_entity_type: str | None
    window_entity_id: str | None
    window_duration: str | None
    window_start: str | None
    fallback_authoritative: bool
    strict: bool
    redis: bool
    redis_get: str | None
    redis_fails: bool
    set_max: float | None
    set_max_fails: bool
    batch: Literal["none", "miss", "amount"]
    batch_amount: float
    memory: str | None
    marker: float | None
    rows: RowsInput | None


class ScriptedMemory(InMemoryCache):
    def __init__(self) -> None:
        super().__init__()
        self.values: dict[str, float | str] = {}
        self.ttls: dict[str, float | None] = {}

    def current(self, key: str) -> float | str | None:
        return self.values.get(key)

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> float | str | None:
        del kwargs
        return self.current(key)

    @override
    def set_cache(self, key: str, value: float | str, **kwargs: float | None) -> None:
        self.values[key] = value
        self.ttls[key] = kwargs.get("ttl")


@dataclass
class Row:
    spend: float
    window_start: datetime | None


@dataclass
class ScriptedTable:
    row: Row | None
    fails: bool
    memory: ScriptedMemory
    reset_marker: tuple[str, float] | None

    async def find_unique(self, where: dict[str, JsonValue]) -> Row | None:
        del where
        if self.fails:
            raise ValueError("the scripted store read failed")
        if self.reset_marker is not None:
            self.memory.set_cache(self.reset_marker[0], self.reset_marker[1])
        return self.row

    async def group_by(self, **kwargs: JsonValue) -> list[dict[str, JsonValue]]:
        del kwargs
        if self.fails:
            raise ValueError("the scripted store read failed")
        return []


class FakeDb(PrismaWrapper):
    def __init__(self, tables: dict[str, ScriptedTable], primary: ScriptedTable) -> None:
        self.tables = tables
        self.primary = primary

    @override
    def __getattr__(self, name: str) -> ScriptedTable:
        if name in self.tables:
            return self.tables[name]
        return self.primary


class FakeClient(PrismaClient):
    def __init__(self, db: FakeDb) -> None:
        self.db = db


class ScriptedRedis(RedisCache):
    # RedisCache.__init__ opens a real client, so skip it and script every call below
    def __init__(self, data: Input) -> None:
        object.__init__(self)
        self.data = data
        self.set_max_calls: list[tuple[str, float]] = []

    @override
    async def async_get_cache(
        self,
        key: str,
        parent_otel_span: Span | None = None,
        **kwargs: JsonValue,
    ) -> str | None:
        del key, parent_otel_span, kwargs
        if self.data["redis_fails"]:
            raise ValueError("the scripted redis read failed")
        return self.data["redis_get"]

    @override
    async def async_set_cache(self, key: str, value: float, **kwargs: JsonValue) -> bool:
        del key, value, kwargs
        return True

    @override
    async def async_set_max(self, key: str, value: float, ttl: int | None = None) -> float | None:
        del ttl
        self.set_max_calls.append((key, value))
        if self.data["set_max_fails"]:
            raise ValueError("the scripted redis set max failed")
        return self.data["set_max"]

    @override
    async def async_batch_get_cache(
        self,
        key_list: list[str] | list[str | None],
        parent_otel_span: Span | None = None,
    ) -> dict[str, float | None]:
        del parent_otel_span
        amount = self.data["batch_amount"] if self.data["batch"] == "amount" else None
        return {key: amount for key in key_list if key is not None}


def build_client(data: Input, memory: ScriptedMemory, start: datetime | None) -> FakeClient | None:
    rows = data["rows"]
    if rows is None:
        return None
    marker_key = f"spend_db_floor:{data['counter_key']}"
    reset = None if rows["reset_marker"] is None else (marker_key, rows["reset_marker"])
    primary = None if rows["primary"] is None else Row(rows["primary"], None)
    end_user = None if rows["end_user"] is None else Row(rows["end_user"], None)
    window = None if rows["window"] is None else Row(rows["window"], start)
    fails = rows["fails"]
    tables = {
        "litellm_endusertable": ScriptedTable(end_user, fails, memory, reset),
        "litellm_budgetwindowspend": ScriptedTable(window, fails, memory, None),
        "litellm_spendlogs": ScriptedTable(None, fails, memory, None),
    }
    return FakeClient(FakeDb(tables, ScriptedTable(primary, fails, memory, reset)))


async def call(data: Input, start: datetime | None) -> JsonValue:
    if data["op"] == "read_spend_counter_cache_value":
        value, authoritative = await proxy_server.read_spend_counter_cache_value(
            data["counter_key"]
        )
        return {"value": value, "authoritative": authoritative}
    spend = await proxy_server.get_current_spend(
        counter_key=data["counter_key"],
        fallback_spend=data["fallback_spend"],
        max_budget=data["max_budget"],
        window_entity_type=data["window_entity_type"],
        window_entity_id=data["window_entity_id"],
        window_duration=data["window_duration"],
        window_start=start,
        fallback_authoritative=data["fallback_authoritative"],
    )
    return {"spend": spend}


def start_of(text: str | None) -> datetime | None:
    if text is None:
        return None
    return datetime.fromisoformat(text)


async def scenario(data: Input, memory: ScriptedMemory, redis: ScriptedRedis) -> JsonValue:
    start = start_of(data["window_start"])
    key = data["counter_key"]
    batch_after: JsonValue = None
    if data["redis"] and data["batch"] != "none":
        with spend_counter_batch_scope(redis, frozenset((key,))):
            result = await call(data, start)
            active = active_spend_counter_batch()
            state = None if active is None else await active.read(key)
            batch_after = None if state is None else {"value": state[0]}
    else:
        result = await call(data, start)
    marker_key = f"spend_db_floor:{key}"
    set_max_calls: list[JsonValue] = []
    for counter, amount in redis.set_max_calls:
        pair: JsonValue = [counter, amount]
        set_max_calls.append(pair)
    marker_ttl = memory.ttls.get(marker_key)
    return {
        "result": result,
        "memory_after": memory.current(key),
        "marker_after": memory.current(marker_key),
        "marker_ttl": marker_ttl,
        "set_max_calls": set_max_calls,
        "batch_after": batch_after,
    }


def run(input_value: Input) -> JsonValue:
    memory = ScriptedMemory()
    if input_value["memory"] is not None:
        memory.set_cache(input_value["counter_key"], input_value["memory"])
    if input_value["marker"] is not None:
        memory.set_cache(f"spend_db_floor:{input_value['counter_key']}", input_value["marker"])
    redis = ScriptedRedis(input_value)
    start = start_of(input_value["window_start"])
    client = build_client(input_value, memory, start)
    # each helper call runs in its own process, so the proxy globals get set in place
    proxy_server.spend_counter_cache.in_memory_cache = memory
    proxy_server.spend_counter_cache.redis_cache = redis if input_value["redis"] else None
    proxy_server.prisma_client = client
    settings: dict[str, bool] = {}
    if input_value["strict"]:
        settings["fail_closed_budget_enforcement"] = True
    proxy_server.general_settings = settings
    return asyncio.run(scenario(input_value, memory, redis))
