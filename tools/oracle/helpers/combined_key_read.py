import asyncio
import sys
import types
from datetime import datetime, timezone
from typing import Final, Literal, Protocol, TypedDict

import litellm
from fastapi import HTTPException
from litellm.proxy import proxy_server
from litellm.proxy import utils as proxy_utils
from litellm.proxy._types import LiteLLM_VerificationTokenView
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.routing_prisma_wrapper import RoutingPrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True


class ScriptedDatabaseError(Exception):
    pass


# log_db_metrics imports PrismaError lazily, so the scripted failure counts as a database failure
fake_prisma = types.ModuleType("prisma")
fake_prisma_errors = types.ModuleType("prisma.errors")
setattr(fake_prisma_errors, "PrismaError", ScriptedDatabaseError)
setattr(fake_prisma, "errors", fake_prisma_errors)
sys.modules["prisma"] = fake_prisma
sys.modules["prisma.errors"] = fake_prisma_errors


class DeprecatedRowFields(TypedDict):
    active_token_id: str
    revoke_at_seconds: int


class Fault(TypedDict):
    type: str
    message: str


class FaultStep(TypedDict):
    error: Fault


class DeprecatedData(TypedDict):
    data: DeprecatedRowFields


class RowData(TypedDict):
    data: dict[str, JsonValue]


RowStep = RowData | FaultStep | None
DeprecatedStep = DeprecatedData | FaultStep | None
ReconnectStep = bool | None


class Input(TypedDict):
    now: str
    step_millis: int
    token: str | None
    query_type: Literal["find_unique", "find_all"]
    check_deprecated: bool
    use_writer: bool
    rows: list[RowStep]
    deprecated: list[DeprecatedStep]
    reconnects: list[ReconnectStep]


class ViewFields(TypedDict):
    token: str | None
    team_alias: str | None
    team_models: list[JsonValue]
    team_blocked: bool
    team_member: dict[str, JsonValue] | None
    org_id: str | None
    organization_alias: str | None
    project_alias: str | None
    team_tpm_limit: int | None
    team_member_tpm_limit: int | None
    expires: str | None


VIEW: Final[TypeAdapter[ViewFields]] = TypeAdapter(ViewFields)


class KeyLookup(Protocol):
    async def get_data(
        self,
        token: str | None = None,
        *,
        table_name: Literal["combined_view"],
        query_type: Literal["find_unique", "find_all"],
        check_deprecated: bool,
        use_writer: bool,
    ) -> LiteLLM_VerificationTokenView | None: ...


class StaleEngine(Protocol):
    def is_still_live(self, current: PrismaWrapper) -> bool: ...


class Counters:
    def __init__(self) -> None:
        self.rows = 0
        self.deprecated = 0
        self.backoffs = 0
        self.cached_plan_warnings = 0
        self.deprecated_used = 0
        self.hooks = 0


class LogRecorder:
    def __init__(self, counters: Counters) -> None:
        self.counters = counters
        self.messages: list[str] = []

    def debug(self, message: str, *args: object) -> None:
        self.record("debug", message, args)

    def info(self, message: str, *args: object) -> None:
        self.record("info", message, args)

    def warning(self, message: str, *args: object) -> None:
        self.record("warning", message, args)

    def error(self, message: str, *args: object) -> None:
        self.record("error", message, args)

    def record(self, level: str, message: str, args: tuple[object, ...]) -> None:
        text = message % args if args else message
        self.messages.append(text)
        if level == "debug" and text == "Deprecated key used during grace period":
            self.counters.deprecated_used += 1
        if level == "warning" and "cached plan" in text:
            self.counters.cached_plan_warnings += 1


class BackoffRecorder:
    def __init__(self, counters: Counters) -> None:
        self.counters = counters

    def __call__(self, message: str) -> None:
        if message.startswith("Backing off"):
            self.counters.backoffs += 1


class MetricLog:
    def __init__(self) -> None:
        self.successes = 0
        self.failures = 0

    async def async_service_success_hook(self, **_kwargs: object) -> None:
        self.successes += 1

    async def async_service_failure_hook(self, **_kwargs: object) -> None:
        self.failures += 1


class LoggingObject:
    def __init__(self, counters: Counters) -> None:
        self.service_logging_obj = MetricLog()
        self.counters = counters

    async def failure_handler(self, **_kwargs: object) -> None:
        self.counters.hooks += 1


class DeprecatedRow:
    def __init__(self, fields: DeprecatedRowFields) -> None:
        self.active_token_id = fields["active_token_id"]
        self.revoke_at = datetime.fromtimestamp(fields["revoke_at_seconds"], tz=timezone.utc)


class FakeDeprecated:
    def __init__(self, steps: list[DeprecatedStep], counters: Counters) -> None:
        self.steps = steps
        self.counters = counters

    async def find_first(self, where: dict[str, object]) -> DeprecatedRow | None:
        if "token" not in where:
            return None
        self.counters.deprecated += 1
        step = self.steps.pop(0) if self.steps else None
        if step is None:
            return None
        if "data" in step:
            return DeprecatedRow(step["data"])
        raise ScriptedDatabaseError(step["error"]["message"])


class FakeWrapper(PrismaWrapper):
    def __init__(
        self,
        generation: int,
        rows: list[RowStep],
        deprecated: list[DeprecatedStep],
        counters: Counters,
    ) -> None:
        self._engine_generation = generation
        self.rows = rows
        self.counters = counters
        self.litellm_deprecatedverificationtoken = FakeDeprecated(deprecated, counters)

    async def query_first(self, _sql_query: str, *_args: object) -> dict[str, JsonValue] | None:
        self.counters.rows += 1
        step = self.rows.pop(0) if self.rows else None
        if step is None:
            return None
        if "data" in step:
            return step["data"]
        raise ScriptedDatabaseError(step["error"]["message"])


class FakeRouting(RoutingPrismaWrapper):
    def __init__(self, writer: FakeWrapper, reader: FakeWrapper) -> None:
        super().__init__(writer, reader)


class ReconnectRecorder:
    def __init__(self, steps: list[ReconnectStep], reader: FakeWrapper) -> None:
        self.steps = steps
        self.reader = reader
        self.effects: list[dict[str, JsonValue]] = []

    async def attempt(
        self,
        reason: str,
        *,
        force_recreate: bool = False,
        stale_read_engine: StaleEngine | None = None,
    ) -> bool:
        stale_live = False
        if stale_read_engine is not None:
            stale_live = stale_read_engine.is_still_live(self.reader)
        self.effects.append(
            {
                "reason": reason,
                "force_recreate": force_recreate,
                "timeout_seconds": None,
                "lock_timeout_seconds": None,
                "stale_present": stale_read_engine is not None,
                "stale_live": stale_live,
            }
        )
        step = self.steps.pop(0) if self.steps else None
        return True if step is None else step


class FakeClient(PrismaClient):
    def __init__(self, db: PrismaWrapper | RoutingPrismaWrapper) -> None:
        self.db = db


class Outcome:
    def __init__(
        self,
        result: str,
        view: ViewFields | None,
        error: dict[str, JsonValue] | None,
    ) -> None:
        self.result = result
        self.view = view
        self.error = error


class Session:
    def __init__(self, input_value: Input) -> None:
        self.counters = Counters()
        self.logger = LogRecorder(self.counters)
        self.logging = LoggingObject(self.counters)
        self.reader = FakeWrapper(
            0, list(input_value["rows"]), list(input_value["deprecated"]), self.counters
        )
        self.writer = FakeWrapper(
            0, list(input_value["rows"]), list(input_value["deprecated"]), self.counters
        )
        self.client = FakeClient(FakeRouting(self.writer, self.reader))
        self.recorder = ReconnectRecorder(list(input_value["reconnects"]), self.reader)
        setattr(self.client, "proxy_logging_obj", self.logging)
        setattr(self.client, "attempt_db_reconnect", self.recorder.attempt)

    def install(self) -> None:
        object.__setattr__(proxy_utils, "verbose_proxy_logger", self.logger)
        object.__setattr__(proxy_utils, "print_verbose", BackoffRecorder(self.counters))
        object.__setattr__(proxy_server, "proxy_logging_obj", self.logging)

    def report(self, outcome: Outcome) -> dict[str, JsonValue]:
        view: dict[str, JsonValue] | None = None
        if outcome.view is not None:
            view = view_report(outcome.view)
        return {
            "result": outcome.result,
            "view": view,
            "error": outcome.error,
            "row_queries": self.counters.rows,
            "deprecated_queries": self.counters.deprecated,
            "reconnects": self.recorder.effects,
            "metric_success": self.logging.service_logging_obj.successes,
            "metric_failure": self.logging.service_logging_obj.failures,
            "hooks": self.counters.hooks,
            "backoffs": self.counters.backoffs,
            "cached_plan_warnings": self.counters.cached_plan_warnings,
            "deprecated_used": self.counters.deprecated_used > 0,
        }


def view_report(view: ViewFields) -> dict[str, JsonValue]:
    return {
        "token": view["token"],
        "team_alias": view["team_alias"],
        "team_models": view["team_models"],
        "team_blocked": view["team_blocked"],
        "team_member": view["team_member"],
        "org_id": view["org_id"],
        "organization_alias": view["organization_alias"],
        "project_alias": view["project_alias"],
        "team_tpm_limit": view["team_tpm_limit"],
        "team_member_tpm_limit": view["team_member_tpm_limit"],
        "expires": view["expires"],
    }


async def call(lookup: KeyLookup, input_value: Input) -> Outcome:
    try:
        found = await lookup.get_data(
            token=input_value["token"],
            table_name="combined_view",
            query_type=input_value["query_type"],
            check_deprecated=input_value["check_deprecated"],
            use_writer=input_value["use_writer"],
        )
    except HTTPException as failure:
        return Outcome(
            "missing_token", None, {"kind": "missing_token", "status": failure.status_code}
        )
    except ScriptedDatabaseError:
        return Outcome("database", None, {"kind": "database", "status": None})
    if found is None:
        return Outcome("none", None, None)
    return Outcome("found", VIEW.validate_json(found.model_dump_json()), None)


async def drain() -> None:
    for _ in range(8):
        await asyncio.sleep(0)


async def scenario(input_value: Input) -> dict[str, JsonValue]:
    session = Session(input_value)
    session.install()
    outcome = await call(session.client, input_value)
    await drain()
    return session.report(outcome)


def run(input_value: Input) -> JsonValue:
    return asyncio.run(scenario(input_value))
