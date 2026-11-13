import asyncio
import os
import time
from collections.abc import Awaitable, Callable
from typing import Literal, Protocol, TypedDict

from litellm._logging import verbose_proxy_logger
from litellm.proxy import utils as proxy_utils
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.routing_prisma_wrapper import RoutingPrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue as Json
from typing_extensions import override

DATABASE_URL_ENV = "DATABASE_URL"
FAILED_MESSAGE = "Prisma DB reconnect failed (%d consecutive). reason=%s error=%s"
TIMEOUT_ERROR = "the database reconnect timed out"
LogArgument = str | int | float | BaseException


# the base passes prisma's HttpConfig through this slot, and this helper never reads it
class OpaqueHttpClient(Protocol): ...


class Settings(TypedDict):
    cooldown_seconds: int
    escalation_threshold: int
    watchdog_timeout_ms: int
    auth_timeout_ms: int
    auth_lock_timeout_ms: int


class Outcome(TypedDict):
    value: bool | None
    error: str | None
    pending: bool


class Request(TypedDict):
    reason: str
    force: bool
    force_recreate: bool
    timeout_ms: int | None
    lock_timeout_ms: int | None


class Target(TypedDict):
    target: Literal["writer", "reader"]
    generation: int


class Generations(TypedDict):
    writer: int
    reader: int


class LockInput(TypedDict):
    mode: Literal["free", "busy", "error"]
    error: str | None


class Scenario(TypedDict):
    name: str
    settings: Settings
    clock: list[float]
    last_attempt: float
    failures: int
    dead: bool
    failed: list[Target]
    database_url: str | None
    generations: Generations
    reader_unavailable: bool
    routing: bool
    engine_state: Literal["alive", "dead", "unknown"]
    bump_writer_generation_on_probe: bool
    request: Request
    stale: Target | None
    lock: LockInput
    recreate: Outcome
    writer_read_only: Outcome
    select_one: Outcome


class Input(TypedDict):
    scenarios: list[Scenario]


# A new or renamed LiteLLM message fails the helper instead of dropping its effect.
LOGGED_FIELDS: dict[str, tuple[str, ...]] = {
    "Skipping DB reconnect attempt inside lock due to cooldown. reason=%s": ("reason",),
    "Skipping DB reconnect attempt due to cooldown. reason=%s": ("reason",),
    "Escalating to heavy reconnect after %d consecutive failures. reason=%s": (
        "consecutive",
        "reason",
    ),
    "Attempting Prisma DB reconnect. reason=%s": ("reason",),
    "Prisma DB reconnect succeeded. reason=%s": ("reason",),
    "Prisma DB reconnect declined. reason=%s detail=%s": ("reason",),
    FAILED_MESSAGE: (
        "consecutive",
        "reason",
        "error",
    ),
    "Skipping DB reconnect attempt due to lock acquisition timeout. reason=%s timeout=%ss": (
        "reason",
        "timeout",
    ),
    "Skipping DB reconnect attempt due to lock acquisition error. reason=%s error=%s": (
        "reason",
        "error",
    ),
    "prisma-query-engine PID %s is dead; reconnecting.": (),
    "Performing Prisma DB reconnect (engine alive or unknown).": (),
    "DATABASE_URL not set; cannot recreate Prisma client.": (),
    "DATABASE_URL not set; cannot reconnect Prisma client.": (),
    (
        "Writer answers the probe but its session is read-only "
        "(writes fail with SQLSTATE 25006); recreating Prisma client."
    ): (),
    (
        "Writer healthy on probe; skipping recreate "
        "(engine likely already replaced by a token refresh)."
    ): (),
    "Writer probe failed (%s); recreating Prisma client.": ("error",),
}


def logged_fields(message: str, args: tuple[LogArgument, ...]) -> dict[str, Json]:
    names = LOGGED_FIELDS[message]
    fields: dict[str, Json] = {}
    for name, argument in zip(names, args, strict=False):
        text = str(argument)
        # asyncio's TimeoutError formats as an empty string, and betterlite names its own timeout
        if message == FAILED_MESSAGE and text == "":
            text = TIMEOUT_ERROR
        fields[name] = text
    return fields


class Recorder:
    def __init__(self, effects: list[Json]) -> None:
        self.effects = effects

    def debug(self, message: str, *args: LogArgument) -> None:
        fields = logged_fields(message, args)
        self.effects.append({"effect": "log", "level": "debug", "fields": fields})

    def info(self, message: str, *args: LogArgument) -> None:
        fields = logged_fields(message, args)
        self.effects.append({"effect": "log", "level": "info", "fields": fields})

    def warning(self, message: str, *args: LogArgument) -> None:
        fields = logged_fields(message, args)
        self.effects.append({"effect": "log", "level": "warn", "fields": fields})

    def error(self, message: str, *args: LogArgument) -> None:
        fields = logged_fields(message, args)
        self.effects.append({"effect": "log", "level": "error", "fields": fields})


class ScriptedTime:
    def __init__(self, values: list[float]) -> None:
        self.values = values
        self.reads = 0

    def time(self) -> float:
        index = min(self.reads, len(self.values) - 1)
        self.reads += 1
        return self.values[index]


class FakeWrapper(PrismaWrapper):
    def __init__(
        self,
        generation: int,
        recreation: Outcome,
        selection: Outcome,
        effects: list[Json],
    ) -> None:
        self._engine_generation = generation
        self.recreation = recreation
        self.selection = selection
        self.effects = effects

    @override
    async def recreate_prisma_client(
        self,
        new_db_url: str,
        http_client: OpaqueHttpClient | None = None,
        *,
        expected_generation: int | None = None,
    ) -> bool:
        self.effects.append(
            {"effect": "recreate", "expected_generation": expected_generation}
        )
        if self.recreation["pending"]:
            await asyncio.sleep(3600)
        if self.recreation["error"] is not None:
            raise RuntimeError(self.recreation["error"])
        return self.recreation["value"] is True

    async def query_raw(self, _query: str) -> list[dict[str, Json]]:
        self.effects.append({"effect": "select_one"})
        if self.selection["pending"]:
            await asyncio.sleep(3600)
        if self.selection["error"] is not None:
            raise RuntimeError(self.selection["error"])
        return []

    def bump_generation(self) -> None:
        self._engine_generation += 1


class FakeRouting(RoutingPrismaWrapper):
    def __init__(
        self,
        writer: PrismaWrapper,
        reader: PrismaWrapper,
        recreation: Outcome,
        effects: list[Json],
        *,
        reader_unavailable: bool,
    ) -> None:
        super().__init__(writer, reader)
        self.recreation = recreation
        self.effects = effects
        self._reader_unavailable = reader_unavailable

    @override
    def mark_writer_recovered(self) -> None:
        self.effects.append({"effect": "mark_writer_recovered"})
        super().mark_writer_recovered()

    @override
    async def recreate_prisma_client(
        self,
        new_db_url: str,
        http_client: OpaqueHttpClient | None = None,
        *,
        expected_generation: int | None = None,
    ) -> bool:
        self.effects.append(
            {"effect": "recreate", "expected_generation": expected_generation}
        )
        if self.recreation["pending"]:
            await asyncio.sleep(3600)
        if self.recreation["error"] is not None:
            raise RuntimeError(self.recreation["error"])
        return self.recreation["value"] is True


class ScriptedLock(asyncio.Lock):
    def __init__(self, script: LockInput, effects: list[Json]) -> None:
        super().__init__()
        self.mode = script["mode"]
        self.error = script["error"]
        self.effects = effects

    @override
    async def acquire(self) -> Literal[True]:
        self.effects.append({"effect": "lock_acquire"})
        if self.mode == "error":
            raise RuntimeError(self.error or "the reconnect lock is unavailable")
        if self.mode == "busy":
            await asyncio.sleep(3600)
        return await super().acquire()

    @override
    def release(self) -> None:
        self.effects.append({"effect": "lock_release"})
        super().release()


class Stale:
    def __init__(self, wrapper: PrismaWrapper, generation: int) -> None:
        self.wrapper = wrapper
        self.generation = generation

    def is_still_live(self, current: PrismaWrapper) -> bool:
        if self.wrapper is not current:
            return False
        return self.generation == current.engine_generation


def wrapper_for(
    target: Literal["writer", "reader"],
    writer: FakeWrapper,
    reader: FakeWrapper,
    *,
    routing: bool,
) -> FakeWrapper:
    if target == "writer" or not routing:
        return writer
    return reader


def failed_repairs(
    entry: Scenario,
    writer: FakeWrapper,
    reader: FakeWrapper,
    *,
    routing: bool,
) -> dict[PrismaWrapper, int]:
    repairs: dict[PrismaWrapper, int] = {}
    for repair in entry["failed"]:
        target = wrapper_for(repair["target"], writer, reader, routing=routing)
        repairs[target] = repair["generation"]
    return repairs


class FakeClient(PrismaClient):
    def __init__(
        self,
        entry: Scenario,
        db: PrismaWrapper | RoutingPrismaWrapper,
        writer: FakeWrapper,
        reader: FakeWrapper,
        effects: list[Json],
    ) -> None:
        settings = entry["settings"]
        self.db = db
        self.effects = effects
        self._db_reconnect_lock = ScriptedLock(entry["lock"], effects)
        self._db_last_reconnect_attempt_ts = entry["last_attempt"]
        self._db_reconnect_cooldown_seconds = settings["cooldown_seconds"]
        self._db_watchdog_reconnect_timeout_seconds = settings["watchdog_timeout_ms"] / 1000
        self._db_auth_reconnect_timeout_seconds = settings["auth_timeout_ms"] / 1000
        self._db_auth_reconnect_lock_timeout_seconds = settings["auth_lock_timeout_ms"] / 1000
        self._consecutive_reconnect_failures = entry["failures"]
        self._failed_recreate_generations = failed_repairs(
            entry, writer, reader, routing=entry["routing"]
        )
        self._reconnect_escalation_threshold = settings["escalation_threshold"]
        self._engine_confirmed_dead = entry["dead"]
        self._engine_pid = 0 if entry["engine_state"] == "unknown" else 1234
        self._engine_state = entry["engine_state"]
        self._probe = entry["writer_read_only"]
        self._probe_target = writer
        self._bump_on_probe = entry["bump_writer_generation_on_probe"]

    @override
    def _is_engine_alive(self) -> bool:
        return self._engine_state != "dead"

    @override
    def _cleanup_engine_watcher(self) -> None:
        self.effects.append({"effect": "cleanup"})

    @override
    async def _start_engine_watcher(self) -> None:
        self.effects.append({"effect": "rearm"})

    @override
    async def _writer_is_read_only(self, writer: PrismaWrapper) -> bool:
        self.effects.append({"effect": "probe"})
        if self._bump_on_probe:
            self._probe_target.bump_generation()
        if self._probe["pending"]:
            await asyncio.sleep(3600)
        if self._probe["error"] is not None:
            raise RuntimeError(self._probe["error"])
        return self._probe["value"] is True

    def failure_count(self) -> int:
        return self._consecutive_reconnect_failures

    def engine_dead(self) -> bool:
        return self._engine_confirmed_dead

    def failed_for(self, wrapper: PrismaWrapper) -> bool:
        if wrapper not in self._failed_recreate_generations:
            return False
        repair = self._failed_recreate_generations[wrapper]
        return repair == wrapper.engine_generation


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


def run(input_value: Input) -> Json:
    scenarios = input_value["scenarios"]
    return [scenario(entry) for entry in scenarios]
