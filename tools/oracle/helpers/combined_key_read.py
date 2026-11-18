import asyncio
import json
import sys
from datetime import UTC, datetime
from types import ModuleType
from typing import Final, Literal, Protocol, TypedDict

import litellm
from litellm._service_logger import ServiceLogging
from litellm.proxy import proxy_server, utils as proxy_utils
from litellm.proxy._types import LiteLLM_VerificationTokenView
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.routing_prisma_wrapper import RoutingPrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

litellm.suppress_debug_info = True


class ScriptedDatabaseError(Exception):
    pass


# log_db_metrics imports PrismaError lazily, so the scripted failure counts as a database failure
PRISMA_ERRORS = ModuleType("prisma.errors")
object.__setattr__(PRISMA_ERRORS, "PrismaError", ScriptedDatabaseError)
sys.modules["prisma"] = ModuleType("prisma")
sys.modules["prisma.errors"] = PRISMA_ERRORS


class DeprecatedRowFields(TypedDict):
    active_token_id: str
    revoke_at_seconds: int


class FailureStep(TypedDict):
    kind: Literal["failure"]
    error: dict[str, str]


class RowData(TypedDict):
    kind: Literal["data"]
    data: dict[str, JsonValue]


class DeprecatedData(TypedDict):
    kind: Literal["data"]
    data: DeprecatedRowFields


RowStep = RowData | FailureStep | None
DeprecatedStep = DeprecatedData | FailureStep | None


class Input(TypedDict):
    token: str | None
    query_type: Literal["find_unique", "find_all"]
    check_deprecated: bool
    use_writer: bool
    rows: list[RowStep]
    deprecated: list[DeprecatedStep]
    reconnects: list[bool | None]


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


class Report(TypedDict):
    result: str
    view: ViewFields | None
    error: dict[str, JsonValue] | None
    row_queries: int
    deprecated_queries: int
    reconnects: list[dict[str, JsonValue]]
    metric_success: int
    metric_failure: int
    hooks: int
    backoffs: int
    cached_plan_warnings: int
    deprecated_used: bool


VIEW: Final[TypeAdapter[ViewFields]] = TypeAdapter(ViewFields)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class KeyLookup(Protocol):
    async def get_data(
        self,
        token: str | None = None, *,
        table_name: Literal["combined_view"],
        query_type: Literal["find_unique", "find_all"],
        check_deprecated: bool,
        use_writer: bool,
    ) -> LiteLLM_VerificationTokenView | None: ...


class StaleEngine(Protocol):
    def is_still_live(self, current: PrismaWrapper) -> bool: ...


class RecorderHooks(Protocol):
    def debug(self, message: str, *args: JsonValue) -> None: ...

    def info(self, message: str, *args: JsonValue) -> None: ...

    def warning(self, message: str, *args: JsonValue) -> None: ...

    def error(self, message: str, *args: JsonValue) -> None: ...

    async def failure_handler(self, **_kwargs: JsonValue) -> None: ...


class Recorder(RecorderHooks):
    def __init__(self) -> None:
        self.rows = 0
        self.deprecated = 0
        self.backoffs = 0
        self.cached_plan_warnings = 0
        self.deprecated_used = 0
        self.hooks = 0
        self.service_logging_obj = ServiceLogging(mock_testing=True)

    @override
    def debug(self, message: str, *args: JsonValue) -> None:
        self.record("debug", message, args)

    @override
    def info(self, message: str, *args: JsonValue) -> None:
        self.record("info", message, args)

    @override
    def warning(self, message: str, *args: JsonValue) -> None:
        self.record("warning", message, args)

    @override
    def error(self, message: str, *args: JsonValue) -> None:
        self.record("error", message, args)

    @override
    async def failure_handler(self, **_kwargs: JsonValue) -> None:
        self.hooks += 1

    def __call__(self, message: str) -> None:
        if message.startswith("Backing off"):
            self.backoffs += 1

    def record(self, level: str, message: str, args: tuple[JsonValue, ...]) -> None:
        text = message % args if args else message
        if level == "debug" and text == "Deprecated key used during grace period":
            self.deprecated_used += 1
        if level == "warning" and "cached plan" in text:
            self.cached_plan_warnings += 1


class DeprecatedRow:
    def __init__(self, fields: DeprecatedRowFields) -> None:
        self.active_token_id = fields["active_token_id"]
        self.revoke_at = datetime.fromtimestamp(fields["revoke_at_seconds"], tz=UTC)


class FakeDeprecated:
    def __init__(self, steps: list[DeprecatedStep], recorder: Recorder) -> None:
        self.steps = steps
        self.recorder = recorder

    async def find_first(self, where: dict[str, JsonValue]) -> DeprecatedRow | None:
        if "token" not in where:
            return None
        self.recorder.deprecated += 1
        step = self.steps.pop(0) if self.steps else None
        if step is None:
            return None
        if step["kind"] == "failure":
            raise ScriptedDatabaseError(step["error"]["message"])
        return DeprecatedRow(step["data"])


class FakeWrapper(PrismaWrapper):
    def __init__(self, rows: list[RowStep], deprecated: list[DeprecatedStep], recorder: Recorder) -> None:
        self._engine_generation = 0
        self.rows = rows
        self.recorder = recorder
        self.litellm_deprecatedverificationtoken = FakeDeprecated(deprecated, recorder)

    async def query_first(self, _sql_query: str, *_args: JsonValue) -> dict[str, JsonValue] | None:
        self.recorder.rows += 1
        step = self.rows.pop(0) if self.rows else None
        if step is None:
            return None
        if step["kind"] == "failure":
            raise ScriptedDatabaseError(step["error"]["message"])
        return step["data"]


class ReconnectRecorder:
    def __init__(self, steps: list[bool | None], reader: FakeWrapper) -> None:
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


class Session:
    def __init__(self, input_value: Input) -> None:
        self.recorder = Recorder()
        self.reader = FakeWrapper(list(input_value["rows"]), list(input_value["deprecated"]), self.recorder)
        self.writer = FakeWrapper(list(input_value["rows"]), list(input_value["deprecated"]), self.recorder)
        self.reconnects = ReconnectRecorder(list(input_value["reconnects"]), self.reader)
        self.client = FakeClient(RoutingPrismaWrapper(self.writer, self.reader))

    def install(self) -> None:
        object.__setattr__(proxy_utils, "verbose_proxy_logger", self.recorder)
        object.__setattr__(proxy_utils, "print_verbose", self.recorder)
        object.__setattr__(proxy_server, "proxy_logging_obj", self.recorder)
        object.__setattr__(self.client, "proxy_logging_obj", self.recorder)
        object.__setattr__(self.client, "attempt_db_reconnect", self.reconnects.attempt)

    async def execute(self, input_value: Input) -> Report:
        result, view, error = await call(self.client, input_value)
        for _ in range(8):
            await asyncio.sleep(0)
        return {
            "result": result,
            "view": view,
            "error": error,
            "row_queries": self.recorder.rows,
            "deprecated_queries": self.recorder.deprecated,
            "reconnects": self.reconnects.effects,
            "metric_success": self.recorder.service_logging_obj.mock_testing_async_success_hook,
            "metric_failure": self.recorder.service_logging_obj.mock_testing_async_failure_hook,
            "hooks": self.recorder.hooks,
            "backoffs": self.recorder.backoffs,
            "cached_plan_warnings": self.recorder.cached_plan_warnings,
            "deprecated_used": self.recorder.deprecated_used > 0,
        }


async def call(lookup: KeyLookup, input_value: Input) -> tuple[str, ViewFields | None, dict[str, JsonValue] | None]:
    try:
        found = await lookup.get_data(
            token=input_value["token"],
            table_name="combined_view",
            query_type=input_value["query_type"],
            check_deprecated=input_value["check_deprecated"],
            use_writer=input_value["use_writer"],
        )
    except HTTPException as failure:
        return "missing_token", None, {"kind": "missing_token", "status": failure.status_code}
    except ScriptedDatabaseError:
        return "database", None, {"kind": "database", "status": None}
    if found is None:
        return "none", None, None
    return "found", VIEW.validate_json(found.model_dump_json()), None


def run(input_value: Input) -> JsonValue:
    session = Session(input_value)
    session.install()
    return JSON.validate_json(json.dumps(asyncio.run(session.execute(input_value))))
