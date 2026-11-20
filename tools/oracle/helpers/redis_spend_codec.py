import asyncio
from collections.abc import Sequence
from datetime import datetime
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict
from unittest.mock import patch

from litellm.caching.redis_cache import RedisCache
from litellm.constants import (
    REDIS_DAILY_TAG_SPEND_UPDATE_BUFFER_KEY,
    REDIS_SPEND_LOGS_BUFFER_KEY,
)
from litellm.proxy import proxy_server
from litellm.proxy.db.db_transaction_queue.redis_update_buffer import RedisUpdateBuffer
from litellm.types.caching import RedisPipelineLpopOperation
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
INVALID: Final[dict[str, JsonValue]] = {"invalid": True}
# get_spend_logs_from_redis_buffer takes its limit from the caller, and a scripted pop ignores it.
LIMIT: Final = 10

Row: TypeAlias = dict[str, JsonValue | datetime]


class Reply(TypedDict):
    kind: Literal["missing", "scalar", "items"]
    value: NotRequired[str]
    values: NotRequired[list[str]]


class RowInput(TypedDict):
    values: dict[str, JsonValue]
    datetimes: dict[str, str]


class ParkStep(TypedDict):
    rows: list[RowInput]


class DrainStep(TypedDict):
    reply: Reply


class PipelineStep(TypedDict):
    replies: list[Reply]


class TagsStep(TypedDict):
    reply: Reply


class Input(TypedDict):
    enabled: bool
    attached: bool
    park: NotRequired[ParkStep]
    drain: NotRequired[DrainStep]
    pipeline: NotRequired[PipelineStep]
    tags: NotRequired[TagsStep]


def reply_text(reply: Reply) -> str:
    value = reply.get("value")
    if value is None:
        raise ValueError("a scalar reply carries no value")
    return value


def reply_items(reply: Reply) -> list[str]:
    values = reply.get("values")
    if values is None:
        raise ValueError("an items reply carries no values")
    return values


def reply_value(reply: Reply) -> str | list[str] | None:
    kind = reply["kind"]
    if kind == "missing":
        return None
    if kind == "scalar":
        return reply_text(reply)
    return reply_items(reply)


# A pipelined pop is typed list[str] | None, so a scripted scalar fails the drain the way betterlite's typed reply does.
def pipeline_reply(reply: Reply) -> list[str] | None:
    kind = reply["kind"]
    if kind == "missing":
        return None
    if kind == "scalar":
        raise ValueError("a pipelined pop answers with a list or nothing")
    return reply_items(reply)


class ScriptedRedis(RedisCache):
    # RedisCache.__init__ opens a real client, so skip it and answer every list call from the script
    def __init__(self, pops: dict[str, Reply], pipeline: list[Reply] | None) -> None:
        object.__init__(self)
        self.pops = pops
        self.pipeline = pipeline
        self.pushed: list[str] = []

    def pushed_rows(self) -> list[JsonValue]:
        return [JSON.validate_json(encoded) for encoded in self.pushed]

    @override
    async def async_lpop(
        self,
        key: str,
        count: int | None = None,
        parent_otel_span: Span | None = None,
        **kwargs: JsonValue,
    ) -> str | list[str] | None:
        del count, parent_otel_span, kwargs
        reply = self.pops.get(key)
        return None if reply is None else reply_value(reply)

    @override
    async def async_lpop_pipeline(
        self,
        lpop_list: list[RedisPipelineLpopOperation],
    ) -> list[list[str] | None]:
        del lpop_list
        if self.pipeline is None:
            raise ValueError("the case scripted no pipeline reply")
        return [pipeline_reply(reply) for reply in self.pipeline]

    @override
    async def async_rpush_and_trim(
        self,
        key: str,
        values: Sequence[str | bytes | int | float],
        max_len: int,
    ) -> int:
        del key, max_len
        for value in values:
            self.pushed.append(str(value))
        return len(self.pushed)


def spend_log_row(row: RowInput) -> Row:
    columns: Row = {}
    for field, value in row["values"].items():
        columns[field] = value
    for field, stamp in row["datetimes"].items():
        columns[field] = datetime.fromisoformat(stamp)
    return columns


def scripted_pops(data: Input) -> dict[str, Reply]:
    pops: dict[str, Reply] = {}
    drain = data.get("drain")
    if drain is not None:
        pops[REDIS_SPEND_LOGS_BUFFER_KEY] = drain["reply"]
    tags = data.get("tags")
    if tags is not None:
        pops[REDIS_DAILY_TAG_SPEND_UPDATE_BUFFER_KEY] = tags["reply"]
    return pops


def scripted_pipeline(data: Input) -> list[Reply] | None:
    pipeline = data.get("pipeline")
    return None if pipeline is None else pipeline["replies"]


async def parked(step: ParkStep, buffer: RedisUpdateBuffer, redis: ScriptedRedis) -> JsonValue:
    rows = [spend_log_row(row) for row in step["rows"]]
    stored = await buffer.store_spend_logs_in_redis(rows)
    parked_state: dict[str, JsonValue] = {"parked": stored, "values": redis.pushed_rows()}
    return parked_state


async def drained(buffer: RedisUpdateBuffer) -> JsonValue:
    try:
        rows = await buffer.get_spend_logs_from_redis_buffer(LIMIT)
    except (AttributeError, KeyError, TypeError, ValueError):
        return INVALID
    return JSON.validate_python(list(rows))


async def pipelined(buffer: RedisUpdateBuffer) -> JsonValue:
    try:
        entity, daily_user, daily_team, daily_org, daily_end_user, daily_agent, window = (
            await buffer.get_all_transactions_from_redis_buffer_pipeline()
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return INVALID
    window_value: JsonValue = None if window is None else JSON.validate_python(list(window))
    slots: dict[str, JsonValue] = {
        "entity": JSON.validate_python(entity),
        "daily_user": JSON.validate_python(daily_user),
        "daily_team": JSON.validate_python(daily_team),
        "daily_org": JSON.validate_python(daily_org),
        "daily_end_user": JSON.validate_python(daily_end_user),
        "daily_agent": JSON.validate_python(daily_agent),
        "window": window_value,
    }
    return slots


async def tagged(buffer: RedisUpdateBuffer) -> JsonValue:
    try:
        tags = await buffer.get_all_daily_tag_spend_update_transactions_from_redis_buffer()
    except (AttributeError, KeyError, TypeError, ValueError):
        return INVALID
    return JSON.validate_python(tags)


async def scenario(data: Input, buffer: RedisUpdateBuffer, redis: ScriptedRedis) -> JsonValue:
    output: dict[str, JsonValue] = {}
    park = data.get("park")
    if park is not None:
        output["parked"] = await parked(park, buffer, redis)
    drain = data.get("drain")
    if drain is not None:
        output["drained"] = await drained(buffer)
    pipeline = data.get("pipeline")
    if pipeline is not None:
        output["pipeline"] = await pipelined(buffer)
    tags = data.get("tags")
    if tags is not None:
        output["tags"] = await tagged(buffer)
    return output


def run(input_value: Input) -> JsonValue:
    redis = ScriptedRedis(scripted_pops(input_value), scripted_pipeline(input_value))
    buffer = RedisUpdateBuffer(redis if input_value["attached"] else None)
    settings: dict[str, bool] = {"use_redis_transaction_buffer": input_value["enabled"]}
    with patch.object(proxy_server, "general_settings", settings):
        return asyncio.run(scenario(input_value, buffer, redis))
