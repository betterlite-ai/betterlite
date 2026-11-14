import asyncio
import os
import time
from collections.abc import Awaitable, Callable

from litellm._logging import verbose_proxy_logger
from litellm.proxy import utils as proxy_utils
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.routing_prisma_wrapper import RoutingPrismaWrapper
from pydantic import JsonValue as Json

from .client import FakeClient, wrapper_for
from .contracts import DATABASE_URL_ENV, Scenario, Target
from .fakes import FakeRouting, FakeWrapper, Recorder, ScriptedTime, Stale


def stale_for(
    stale: Target | None,
    writer: FakeWrapper,
    reader: FakeWrapper,
    *,
    routing: bool,
) -> Stale | None:
    if stale is None:
        return None
    wrapper = wrapper_for(stale["target"], writer, reader, routing=routing)
    return Stale(wrapper, stale["generation"])


def seconds(value: int | None) -> float | None:
    if value is None:
        return None
    return value / 1000


def set_database_url(url: str | None) -> None:
    if url is None:
        os.environ.pop(DATABASE_URL_ENV, None)
    else:
        os.environ[DATABASE_URL_ENV] = url


async def run_attempt(
    entry: Scenario,
    callback: Callable[..., Awaitable[bool]],
    stale: Stale | None,
) -> bool:
    request = entry["request"]
    set_database_url(entry["database_url"])
    return await callback(
        reason=request["reason"],
        force=request["force"],
        timeout_seconds=seconds(request["timeout_ms"]),
        lock_timeout_seconds=seconds(request["lock_timeout_ms"]),
        force_recreate=request["force_recreate"],
        stale_read_engine=stale,
    )


def scenario(entry: Scenario) -> Json:
    effects: list[Json] = []
    recreation = entry["recreate"]
    selection = entry["select_one"]
    writer = FakeWrapper(entry["generations"]["writer"], recreation, selection, effects)
    reader = FakeWrapper(entry["generations"]["reader"], recreation, selection, effects)
    routing = entry["routing"]
    db: PrismaWrapper | RoutingPrismaWrapper
    if routing:
        db = FakeRouting(
            writer,
            reader,
            recreation,
            effects,
            reader_unavailable=entry["reader_unavailable"],
        )
    else:
        db = writer
    client = FakeClient(entry, db, writer, reader, effects)
    stale = stale_for(entry["stale"], writer, reader, routing=routing)
    callback: Callable[..., Awaitable[bool]] = client.attempt_db_reconnect
    object.__setattr__(proxy_utils, "verbose_proxy_logger", Recorder(effects))
    object.__setattr__(proxy_utils, "time", ScriptedTime(entry["clock"]))
    try:
        result = asyncio.run(run_attempt(entry, callback, stale))
    finally:
        object.__setattr__(proxy_utils, "verbose_proxy_logger", verbose_proxy_logger)
        object.__setattr__(proxy_utils, "time", time)
    return {
        "name": entry["name"],
        "result": result,
        "effects": effects,
        "failures": client.failure_count(),
        "dead": client.engine_dead(),
        "failed_writer": client.failed_for(writer),
        "failed_reader": client.failed_for(reader if routing else writer),
    }
