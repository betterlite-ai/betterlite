import asyncio
import json
import logging
import sys
from datetime import UTC, datetime
from types import ModuleType
from typing import Final, Generic, Literal, Protocol, TypedDict, TypeVar

from litellm.proxy._types import LiteLLM_VerificationTokenView
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.routing_prisma_wrapper import RoutingPrismaWrapper
from litellm.proxy.utils import PrismaClient, ProxyLogging
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override


class ScriptedDatabaseError(Exception):
    pass


class PrismaModule(ModuleType):
    PrismaError: type[Exception]
    ClientNotConnectedError: type[Exception]


# LiteLLM imports prisma.errors eagerly in log_db_metrics and PrismaWrapper; prisma is absent here
PRISMA: Final = PrismaModule("prisma")
PRISMA.PrismaError = ScriptedDatabaseError
PRISMA.ClientNotConnectedError = ScriptedDatabaseError
sys.modules["prisma"] = PRISMA
sys.modules["prisma.errors"] = PRISMA


class DeprecatedFields(TypedDict):
    active_token_id: str
    revoke_at_seconds: int


class FailureStep(TypedDict):
    kind: Literal["failure"]
    error: dict[str, str]


T = TypeVar("T")


class StepData(TypedDict, Generic[T]):
    kind: Literal["data"]
    data: T


RowStep = StepData[dict[str, JsonValue]] | FailureStep | None
DeprecatedStep = StepData[DeprecatedFields] | FailureStep | None


def take_step(steps: list[StepData[T] | FailureStep | None]) -> T | None:
    step = steps.pop(0) if steps else None
    if step is None:
        return None
    if step["kind"] == "failure":
        raise ScriptedDatabaseError(step["error"]["message"])
    return step["data"]


class Input(TypedDict):
    now: str
    step_millis: int
    token: str | None
    query_type: Literal["find_unique", "find_all"]
    check_deprecated: bool
    use_writer: bool
    rows: list[RowStep]
    deprecated: list[DeprecatedStep]
    reconnects: list[bool | None]


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
VIEW_FIELDS: Final = (
    "token", "team_alias", "team_models", "team_blocked", "team_member", "org_id",
    "organization_alias", "project_alias", "team_tpm_limit", "team_member_tpm_limit", "expires",
)


class DeprecatedRow:
    def __init__(self, fields: DeprecatedFields) -> None:
        self.active_token_id = fields["active_token_id"]
        self.revoke_at = datetime.fromtimestamp(fields["revoke_at_seconds"], tz=UTC)


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


class DeprecatedTable(Protocol):
    async def find_first(self, where: dict[str, JsonValue]) -> DeprecatedRow | None: ...


class TraceHandler(logging.Handler):
    def __init__(self, events: list[str]) -> None:
        super().__init__(level=logging.DEBUG)
        self.events = events

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith("PrismaClient: find_unique for token:"):
            self.events.append("token_hash")
        elif message.startswith("PostgreSQL cached plan error detected"):
            self.events.append("cached_plan")
        elif message.startswith("LiteLLM Prisma Client Exception: Error with `get_data`"):
            self.events.append("failure")
        elif message.startswith("Backing off"):
            self.events.append("backoff")
        elif message == "Deprecated key used during grace period":
            self.events.append("deprecated_used")
        elif message.startswith("Deprecated key lookup skipped:"):
            self.events.append("deprecated_failed")


class Recorder(ProxyLogging):
    def __init__(self, session: "Session") -> None:
        super().__init__(UserApiKeyCache())
        self.session = session

    @override
    async def failure_handler(
        self,
        original_exception: Exception,
        duration: float,
        call_type: str,
        traceback_str: str = "",
    ) -> None:
        self.session.hook_calls.append((duration, str(original_exception), traceback_str))
        self.session.events.append(f"hook:{call_type}")


class FakeDeprecated(DeprecatedTable):
    def __init__(self, session: "Session", *, use_writer: bool) -> None:
        self.session = session
        self.use_writer = use_writer

    # RoutingPrismaWrapper only routes a per-model accessor that also exposes find_many and create
    async def find_many(self) -> list[DeprecatedRow]:
        return []

    async def create(self, data: DeprecatedFields) -> None:
        del data

    @override
    async def find_first(self, where: dict[str, JsonValue]) -> DeprecatedRow | None:
        await self.session.record_query(
            "litellm_deprecatedverificationtoken", str(where["token"]),
            use_writer=self.use_writer, now_seconds=int(self.session.now.timestamp()),
        )
        data = take_step(self.session.deprecated)
        if data is None:
            return None
        row = DeprecatedRow(data)
        # the case clock stands in for the source's wall clock, so both sides filter together
        return None if row.revoke_at <= self.session.now else row


class FakeWrapper(PrismaWrapper):
    def __init__(self, session: "Session", *, use_writer: bool) -> None:
        super().__init__(object())
        self.session = session
        self.use_writer = use_writer
        self.litellm_deprecatedverificationtoken = FakeDeprecated(session, use_writer=use_writer)

    async def query_first(self, _sql_query: str, *args: JsonValue) -> dict[str, JsonValue] | None:
        await self.session.record_query(
            "combined_view", str(args[0]), use_writer=self.use_writer, now_seconds=None,
        )
        return take_step(self.session.rows)


class FakeClient(PrismaClient):
    def __init__(self, db: PrismaWrapper | RoutingPrismaWrapper, session: "Session") -> None:
        self.db = db
        self.session = session
        self.proxy_logging_obj = session.recorder

    @override
    async def attempt_db_reconnect(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L6435
        self,
        reason: str,
        force: bool = False,
        timeout_seconds: float | None = None,
        lock_timeout_seconds: float | None = None,
        force_recreate: bool = False,
        stale_read_engine: StaleEngine | None = None,
    ) -> bool:
        stale: bool | None = None
        if stale_read_engine is not None:
            stale = stale_read_engine.is_still_live(self.session.reader)
        self.session.reconnect_records.append({
            "reason": reason, "force": force, "force_recreate": force_recreate,
            "timeout_seconds": timeout_seconds, "lock_timeout_seconds": lock_timeout_seconds,
            "stale": stale,
        })
        step = self.session.reconnects.pop(0) if self.session.reconnects else None
        return True if step is None else step


class Session:
    def __init__(self, input_value: Input) -> None:
        self.now = datetime.fromisoformat(input_value["now"])
        self.step_millis = input_value["step_millis"]
        self.rows: list[RowStep] = list(input_value["rows"])
        self.deprecated: list[DeprecatedStep] = list(input_value["deprecated"])
        self.reconnects: list[bool | None] = list(input_value["reconnects"])
        self.events: list[str] = []
        self.hook_calls: list[tuple[float, str, str]] = []
        self.queries: list[dict[str, JsonValue]] = []
        self.reconnect_records: list[dict[str, JsonValue]] = []
        self.reader = FakeWrapper(self, use_writer=False)
        self.writer = FakeWrapper(self, use_writer=True)
        self.recorder = Recorder(self)
        self.client = FakeClient(RoutingPrismaWrapper(self.writer, self.reader), self)

    async def record_query(
        self, target: str, token_hash: str, *, use_writer: bool, now_seconds: int | None,
    ) -> None:
        if self.step_millis:
            await asyncio.sleep(self.step_millis / 1000)
        self.queries.append({
            "target": target, "token_hash": token_hash,
            "use_writer": use_writer, "now_seconds": now_seconds,
        })

    async def execute(self, input_value: Input) -> JsonValue:
        result, view, error = await call(self.client, input_value)
        for _ in range(8):
            await asyncio.sleep(0)
        report = {
            "result": result, "view": view, "error": error,
            "queries": self.queries, "reconnects": self.reconnect_records, "events": self.events,
        }
        return JSON.validate_json(json.dumps(report))


async def call(lookup: KeyLookup, input_value: Input) -> tuple[str, JsonValue, JsonValue]:
    try:
        found = await lookup.get_data(
            token=input_value["token"], table_name="combined_view",
            query_type=input_value["query_type"], check_deprecated=input_value["check_deprecated"],
            use_writer=input_value["use_writer"],
        )
    except HTTPException as failure:
        return "missing_token", None, {"kind": "missing_token", "status": failure.status_code}
    except ScriptedDatabaseError:
        return "database", None, {"kind": "database", "status": None}
    if found is None:
        return "none", None, None
    dump = TypeAdapter(dict[str, JsonValue]).validate_json(found.model_dump_json())
    return "found", {field: dump[field] for field in VIEW_FIELDS}, None


def run(input_value: Input) -> JsonValue:
    session = Session(input_value)
    tracer = TraceHandler(session.events)
    logger = logging.getLogger("LiteLLM Proxy")
    saved_handlers = list(logger.handlers)
    saved_level = logger.level
    saved_propagate = logger.propagate
    for handler in saved_handlers:
        logger.removeHandler(handler)
    logger.addHandler(tracer)
    logger.setLevel(logging.DEBUG)
    logger.propagate = False
    try:
        return asyncio.run(session.execute(input_value))
    finally:
        logger.removeHandler(tracer)
        for handler in saved_handlers:
            logger.addHandler(handler)
        logger.setLevel(saved_level)
        logger.propagate = saved_propagate
