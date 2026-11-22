import asyncio
import sys
from datetime import UTC, datetime
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


class ParsePayload(Strict):
    bytes: int = Field(ge=0)


class QueryBounds(Strict):
    start: str
    end: str


class ValidParse(Strict):
    kind: Literal["valid"] = "valid"
    rows: int


class InvalidParse(Strict):
    kind: Literal["invalid"] = "invalid"
    bytes: int
    limit: int


ParseOutput = Annotated[ValidParse | InvalidParse, Field(discriminator="kind")]


class Output(Strict):
    response: dict[str, JsonValue]
    query: QueryBounds
    parse: ParseOutput | None = None


class FakeDb:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows: list[dict[str, JsonValue]] = rows
        self.start: str = ""
        self.end: str = ""

    async def query_raw(self, _query: str, *args: object) -> list[dict[str, JsonValue]]:
        start, end = args[:2]
        if not (isinstance(start, str) and isinstance(end, str)):
            raise ProbeError("invalid_request", "the benchmark query bounds are not date strings")
        self.start = start
        self.end = end
        return self.rows


class FakePrisma:
    def __init__(self, db: FakeDb) -> None:
        self.db: FakeDb = db


class FakeRouter:
    def __init__(self, model_list: list[dict[str, object]]) -> None:
        self.model_list: list[dict[str, object]] = model_list


class ProxyModule(ModuleType):
    def __init__(self, name: str, llm_router: FakeRouter, prisma_client: FakePrisma) -> None:
        super().__init__(name)
        self.llm_router: FakeRouter = llm_router
        self.prisma_client: FakePrisma = prisma_client


def deployment_entry(deployment: Deployment) -> dict[str, object]:
    params: dict[str, object] = {}
    if deployment.model is not None:
        params["model"] = deployment.model
    entry: dict[str, object] = {"litellm_params": params}
    if deployment.model_name is not None:
        entry["model_name"] = deployment.model_name
    return entry


def utc_day(now_unix_millis: int) -> str:
    try:
        instant = datetime.fromtimestamp(now_unix_millis / 1000, tz=UTC)
    except (OverflowError, OSError, ValueError) as error:
        raise ProbeError("invalid_request", "the probe clock is outside the supported calendar") from error
    return instant.date().isoformat()


class BenchmarkAggregates(Operation):
    now_unix_millis: int
    start_date: str | None = None
    end_date: str | None = None
    deployments: list[Deployment] = Field(default_factory=list)
    rows: list[dict[str, JsonValue]] = Field(default_factory=list)
    parse: ParsePayload | None = None

    @override
    def run(self) -> Output:
        end_date = self.end_date if self.end_date is not None else utc_day(self.now_unix_millis)
        db = FakeDb(self.rows)
        module = ProxyModule(
            PROXY_MODULE,
            FakeRouter([deployment_entry(deployment) for deployment in self.deployments]),
            FakePrisma(db),
        )
        previous = sys.modules.get(PROXY_MODULE)
        sys.modules[PROXY_MODULE] = module
        try:
            response = asyncio.run(
                auto_router_endpoints.get_auto_router_benchmarks(
                    user_api_key_dict=UserAPIKeyAuth(user_role=LitellmUserRoles.PROXY_ADMIN),
                    start_date=self.start_date,
                    end_date=end_date,
                )
            )
        except HTTPException as error:
            raise ProbeError("invalid_request", str(error.detail)) from error
        finally:
            if previous is None:
                del sys.modules[PROXY_MODULE]
            else:
                sys.modules[PROXY_MODULE] = previous
        return Output(
            response=RESPONSE.validate_json(response.model_dump_json()),
            query=QueryBounds(start=db.start, end=db.end),
            parse=ValidParse(rows=len(self.rows)) if self.parse is not None else None,
        )
