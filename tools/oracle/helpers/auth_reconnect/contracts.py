from collections.abc import Iterator
from contextlib import contextmanager
from typing import Literal, Protocol, TypedDict

from litellm.proxy import utils as proxy_utils
from pydantic import JsonValue as Json

DATABASE_URL_ENV = "DATABASE_URL"
FAILED_MESSAGE = "Prisma DB reconnect failed (%d consecutive). reason=%s error=%s"
TIMEOUT_ERROR = "the database reconnect timed out"
LogArgument = str | int | float | BaseException


# the base passes prisma's HttpConfig through this slot, and this helper never reads it
class OpaqueHttpClient(Protocol): ...


# the surface of litellm._logging.verbose_proxy_logger that attempt_db_reconnect calls
class ScriptedLogger(Protocol):
    def debug(self, message: str, *args: LogArgument) -> None: ...

    def info(self, message: str, *args: LogArgument) -> None: ...

    def warning(self, message: str, *args: LogArgument) -> None: ...

    def error(self, message: str, *args: LogArgument) -> None: ...


class ScriptedClock(Protocol):
    def time(self) -> float: ...


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
    bump_on_probe: bool
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


@contextmanager
def scripted_proxy(logger: ScriptedLogger, clock: ScriptedClock) -> Iterator[None]:
    original_logger = proxy_utils.verbose_proxy_logger
    original_time = proxy_utils.time
    object.__setattr__(proxy_utils, "verbose_proxy_logger", logger)
    object.__setattr__(proxy_utils, "time", clock)
    try:
        yield
    finally:
        object.__setattr__(proxy_utils, "verbose_proxy_logger", original_logger)
        object.__setattr__(proxy_utils, "time", original_time)
