import asyncio
import sys
from collections.abc import Mapping
from types import ModuleType
from typing import Annotated, Final, Literal

from fastapi import HTTPException
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.management_endpoints import auto_router_endpoints
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
RESPONSE: Final = TypeAdapter(dict[str, JsonValue])


class Deployment(Strict):
    model_name: str | None = None
    model: str | None = None


class QueryBounds(Strict):
    start: str
    end: str
    api_key: str | None = None
    user_id: str | None = None


class SessionLookup(Strict):
    api_key: str
    session_id: str
    matched: bool


class BenchmarksOutcome(Strict):
    kind: Literal["benchmarks"] = "benchmarks"
    response: dict[str, JsonValue]
    query: QueryBounds


class SessionOutcome(Strict):
    kind: Literal["session"] = "session"
    response: dict[str, JsonValue]
    lookup: SessionLookup


class FailedOutcome(Strict):
    kind: Literal["failed"] = "failed"
    status: int


StepOutcome = Annotated[BenchmarksOutcome | SessionOutcome | FailedOutcome, Field(discriminator="kind")]


class BenchmarksStep(Strict):
    kind: Literal["benchmarks"] = "benchmarks"
    role: str | None = None
    database: bool = False
    start_date: str | None = None
    end_date: str | None = None
    api_key: str | None = None
    user_id: str | None = None
    rows: list[dict[str, JsonValue]] = Field(default_factory=list)


class SessionSeed(Strict):
    last_turn_at: str
    row: dict[str, JsonValue]


class SessionStep(Strict):
    kind: Literal["session"] = "session"
    api_key: str = ""
    session_id: str
    sessions: list[SessionSeed] = Field(default_factory=list)


Step = Annotated[BenchmarksStep | SessionStep, Field(discriminator="kind")]


class Output(Strict):
    steps: list[StepOutcome]


class FakeTable:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows: list[dict[str, JsonValue]] = rows
        self.lookup: dict[str, JsonValue] | None = None

    async def find_first(
        self, where: Mapping[str, object], order: Mapping[str, str]
    ) -> dict[str, JsonValue] | None:
        api_key = where.get("api_key")
        session_id = where.get("session_id")
        if not (isinstance(api_key, str) and isinstance(session_id, str)):
            raise ProbeError("invalid_request", "the session lookup keys are not strings")
        matching = [
            row for row in self.rows if row.get("api_key") == api_key and row.get("session_id") == session_id
        ]
        chosen = sorted(matching, key=turn_stamp, reverse=True)[0] if matching else None
        self.lookup = {"api_key": api_key, "session_id": session_id, "matched": chosen is not None}
        return chosen


def turn_stamp(row: dict[str, JsonValue]) -> str:
    value = row.get("last_turn_at")
    return value if isinstance(value, str) else ""


class FakeDb:
    def __init__(self, rows: list[dict[str, JsonValue]], table: FakeTable) -> None:
        self.rows: list[dict[str, JsonValue]] = rows
        self.litellm_autoroutersession: FakeTable = table
        self.start: str = ""
        self.end: str = ""
        self.api_key: str | None = None
        self.user_id: str | None = None

    async def query_raw(self, _query: str, *args: object) -> list[dict[str, JsonValue]]:
        start, end, api_key, user_id = args[:4]
        if not (isinstance(start, str) and isinstance(end, str)):
            raise ProbeError("invalid_request", "the benchmark query bounds are not date strings")
        self.start = start
        self.end = end
        self.api_key = api_key if isinstance(api_key, str) else None
        self.user_id = user_id if isinstance(user_id, str) else None
        return self.rows


class FakePrisma:
    def __init__(self, db: FakeDb) -> None:
        self.db: FakeDb = db


class FakeRouter:
    def __init__(self, model_list: list[dict[str, object]]) -> None:
        self.model_list: list[dict[str, object]] = model_list


class ProxyModule(ModuleType):
    def __init__(self, name: str, llm_router: FakeRouter, prisma_client: FakePrisma | None) -> None:
        super().__init__(name)
        self.llm_router: FakeRouter = llm_router
        self.prisma_client: FakePrisma | None = prisma_client


def deployment_entry(deployment: Deployment) -> dict[str, object]:
    params: dict[str, object] = {}
    if deployment.model is not None:
        params["model"] = deployment.model
    entry: dict[str, object] = {"litellm_params": params}
    if deployment.model_name is not None:
        entry["model_name"] = deployment.model_name
    return entry


class AutoRouterReadApi(Operation):
    now_unix_millis: int
    deployments: list[Deployment] = Field(default_factory=list)
    steps: list[Step] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        outcomes: list[StepOutcome] = []
        for step in self.steps:
            if isinstance(step, BenchmarksStep):
                outcomes.append(self.benchmarks_outcome(step))
            else:
                outcomes.append(self.session_outcome(step))
        return Output(steps=outcomes)

    def benchmarks_outcome(self, step: BenchmarksStep) -> StepOutcome:
        db = FakeDb(step.rows, FakeTable([]))
        prisma = FakePrisma(db) if step.database else None
        module = ProxyModule(
            PROXY_MODULE,
            FakeRouter([deployment_entry(deployment) for deployment in self.deployments]),
            prisma,
        )
        role = LitellmUserRoles(step.role) if step.role is not None else None
        previous = sys.modules.get(PROXY_MODULE)
        sys.modules[PROXY_MODULE] = module
        try:
            response = asyncio.run(
                auto_router_endpoints.get_auto_router_benchmarks(
                    user_api_key_dict=UserAPIKeyAuth(user_role=role),
                    start_date=step.start_date,
                    end_date=step.end_date,
                    api_key=step.api_key,
                    user_id=step.user_id,
                )
            )
        except HTTPException as error:
            return FailedOutcome(status=error.status_code)
        finally:
            restore(previous)
        return BenchmarksOutcome(
            response=RESPONSE.validate_json(response.model_dump_json()),
            query=QueryBounds(start=db.start, end=db.end, api_key=db.api_key, user_id=db.user_id),
        )

    def session_outcome(self, step: SessionStep) -> StepOutcome:
        table = FakeTable([seed.row for seed in step.sessions])
        db = FakeDb([], table)
        module = ProxyModule(PROXY_MODULE, FakeRouter([]), FakePrisma(db))
        previous = sys.modules.get(PROXY_MODULE)
        sys.modules[PROXY_MODULE] = module
        try:
            response = asyncio.run(
                auto_router_endpoints.get_auto_router_session(
                    user_api_key_dict=UserAPIKeyAuth(api_key=step.api_key),
                    session_id=step.session_id,
                )
            )
        except HTTPException as error:
            return FailedOutcome(status=error.status_code)
        finally:
            restore(previous)
        if table.lookup is None:
            raise ProbeError("invalid_request", "the session read never reached the repository")
        return SessionOutcome(
            response=RESPONSE.validate_json(response.model_dump_json()),
            lookup=SessionLookup.model_validate(table.lookup),
        )


def restore(previous: ModuleType | None) -> None:
    if previous is None:
        del sys.modules[PROXY_MODULE]
    else:
        sys.modules[PROXY_MODULE] = previous
